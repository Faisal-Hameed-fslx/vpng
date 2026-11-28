//! The documents the module holds open, and the operations on them.
//!
//! This is the whole of the module's logic, and it is ordinary Rust: the
//! exported functions in `ffi` are a thin, unsafe shell around it. Keeping
//! the logic here means it can be unit-tested on the host, where the
//! WebAssembly build is not available.

use std::cell::RefCell;

use vpng_core::{Error, Limits, PixelFormat, Rect, Result};
use vpng_decoder::decode_tile_payload;
use vpng_source::{ContainerMeta, MemorySource, RangeReader};

/// One open document.
///
/// A document is opened in one of two ways:
///
/// * **Whole**: the whole container is in memory, and the reader fetches the
///   payloads it needs out of it. This is the ordinary case.
/// * **Parts**: only the header and the directory are in memory, and the
///   caller supplies the payloads it has fetched. This is the case a
///   browser is in when it reads a container over HTTP Range requests: it
///   knows the layout after two range requests, and it can then ask for the
///   payload of exactly the tile it wants.
pub enum Document {
    /// The whole container is in memory.
    Whole(RangeReader<MemorySource>),
    /// Only the header and the directory are in memory.
    Parts(ContainerMeta),
}

impl Document {
    /// The container's metadata.
    pub fn meta(&self) -> &ContainerMeta {
        match self {
            Document::Whole(reader) => reader.meta(),
            Document::Parts(meta) => meta,
        }
    }

    /// Decode one rectangle of the original image.
    ///
    /// The bytes are the **canonical** pixels: rows tightly packed, and a
    /// 16-bit sample little-endian — the same order the format stores and
    /// the same order the native decoder returns.
    pub fn decode_region(&mut self, rect: Rect, limits: &Limits) -> Result<Vec<u8>> {
        match self {
            Document::Whole(reader) => Ok(reader.decode_region(rect, limits)?.into_data()),
            Document::Parts(_) => Err(Error::unsupported(
                "this document holds only the header and the directory, so the caller must \
                 supply the tile payloads; decode them one at a time with `decode_payload`",
            )),
        }
    }

    /// Decode one tile payload the caller has fetched.
    ///
    /// The payload's CRC32C is verified against the directory before the
    /// codec sees it, and the decoded length is checked against the
    /// directory's declaration. It is the same check the native decoder
    /// makes, through the same function.
    pub fn decode_payload(&self, index: usize, payload: &[u8], limits: &Limits) -> Result<Vec<u8>> {
        let entry = *self
            .meta()
            .tile_entry(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        if payload.len() != entry.compressed_length as usize {
            return Err(Error::corrupt(format!(
                "tile {index} payload is {} bytes, the directory says {}",
                payload.len(),
                entry.compressed_length
            )));
        }
        decode_tile_payload(index, payload, &entry, self.meta().format(), limits)
    }

    /// Decode one layer's own pixels.
    ///
    /// The result is sized to the **layer's own rectangle**, not the canvas,
    /// and it holds the layer's pixels exactly. A mask layer decodes to its
    /// coverage image: the same pixels, of which only the alpha channel
    /// carries meaning.
    pub fn decode_layer(&mut self, position: usize, limits: &Limits) -> Result<Vec<u8>> {
        match self {
            Document::Whole(reader) => {
                let (rect, level) = {
                    let meta = reader.meta();
                    let layer = meta.layer(position).ok_or_else(|| {
                        Error::invalid(format!("layer {position} is out of range"))
                    })?;
                    (layer.rect(), layer.base_level().clone())
                };
                let format = reader.meta().format();
                let image =
                    vpng_decoder::decode_layer_from(&level, format, rect, limits, |index| {
                        reader.decode_tile(index, limits)
                    })?;
                Ok(image.into_data())
            }
            Document::Parts(_) => Err(Error::unsupported(
                "this document holds only the header and the directory, so the caller must \
                 supply the tile payloads; decode them one at a time with `decode_payload`",
            )),
        }
    }

    /// Decode one rectangle of the **GPU payload**.
    ///
    /// The result is **lossy** — BC1 and BC4 throw information away — and it
    /// is always **RGBA8**, whatever the container's own format is: a GPU
    /// payload is always eight-bit, and a texture is always RGBA.
    ///
    /// Nothing exact reads this path.
    pub fn decode_gpu_region(&mut self, rect: Rect, limits: &Limits) -> Result<Vec<u8>> {
        match self {
            Document::Whole(reader) => {
                let level = reader.meta().base_level().clone();
                let image = vpng_decoder::decode_region_from(
                    &level,
                    PixelFormat::rgba8(),
                    rect,
                    limits,
                    |index| reader.decode_gpu_tile(index),
                )?;
                Ok(image.into_data())
            }
            Document::Parts(_) => Err(Error::unsupported(
                "this document holds only the header and the directory, so the caller must \
                 supply the GPU payloads; fetch them one at a time with `decode_gpu_payload`",
            )),
        }
    }

    /// Decode one tile's **GPU payload** the caller has fetched.
    ///
    /// The payload's CRC32C is verified against the GPU table before the
    /// block decoder sees it, and the decoded length is checked against the
    /// tile's geometry. The result is **lossy** and is always RGBA8.
    pub fn decode_gpu_payload(
        &self,
        index: usize,
        payload: &[u8],
        limits: &Limits,
    ) -> Result<Vec<u8>> {
        let record = *self
            .meta()
            .gpu_table()
            .and_then(|table| table.get(index))
            .ok_or_else(|| Error::unsupported("this file carries no GPU payload table"))?;
        let entry = *self
            .meta()
            .tile_entry(index)
            .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
        if payload.len() != record.payload_length as usize {
            return Err(Error::corrupt(format!(
                "tile {index} GPU payload is {} bytes, the table says {}",
                payload.len(),
                record.payload_length
            )));
        }
        let checksum = vpng_container::crc32c(payload);
        if checksum != record.payload_checksum {
            return Err(Error::corrupt(format!(
                "the GPU payload of tile {index} has checksum {checksum:#010x}, the table says {:#010x}",
                record.payload_checksum
            )));
        }
        let _ = limits;
        vpng_decoder::gpu::decode_gpu_tile(
            &record,
            u32::from(entry.width),
            u32::from(entry.height),
            payload,
        )
    }
}

/// The open documents, addressed by handle.
///
/// A handle is the index into the table plus one, so that zero is always an
/// error rather than a document.
pub struct Registry {
    documents: RefCell<Vec<Option<Document>>>,
}

impl Registry {
    /// A registry with nothing open.
    pub const fn new() -> Self {
        Registry {
            documents: RefCell::new(Vec::new()),
        }
    }

    /// Open a whole container held in memory.
    pub fn open_whole(&self, bytes: Vec<u8>) -> Result<i32> {
        let source = MemorySource::new(bytes);
        let reader = RangeReader::open_source(source, &Limits::default())?;
        self.insert(Document::Whole(reader))
    }

    /// Open a container from its header and its directory alone.
    ///
    /// `file_size` is the size the container claims for itself, which is
    /// what the payload ranges are checked against.
    ///
    /// A container with a layer table needs the table as well, so this
    /// refuses one: use [`Registry::open_parts_with_layers`]. A container
    /// with a GPU payload table needs that table too, so this refuses one:
    /// use [`Registry::open_parts_with_tables`].
    pub fn open_parts(&self, header: &[u8], directory: &[u8], file_size: u64) -> Result<i32> {
        self.open_parts_with_layers(header, directory, None, file_size)
    }

    /// Open a container from its header, its directory and its layer table.
    ///
    /// `layer_table` is the table's bytes, or `None` for a container that has
    /// none.
    pub fn open_parts_with_layers(
        &self,
        header: &[u8],
        directory: &[u8],
        layer_table: Option<&[u8]>,
        file_size: u64,
    ) -> Result<i32> {
        self.open_parts_with_tables(header, directory, layer_table, None, file_size)
    }

    /// Open a container from its header, its directory and its tables.
    ///
    /// `layer_table` and `gpu_table` are the tables' bytes, or `None` for a
    /// container that has none.
    pub fn open_parts_with_tables(
        &self,
        header: &[u8],
        directory: &[u8],
        layer_table: Option<&[u8]>,
        gpu_table: Option<&[u8]>,
        file_size: u64,
    ) -> Result<i32> {
        let meta = ContainerMeta::parse(
            header,
            directory,
            layer_table,
            gpu_table,
            file_size,
            &Limits::default(),
        )?;
        self.insert(Document::Parts(meta))
    }

    /// Store a document and return its handle.
    fn insert(&self, document: Document) -> Result<i32> {
        let mut documents = self.documents.borrow_mut();
        // Reuse a closed slot before growing the table.
        for (index, slot) in documents.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(document);
                return Ok(index as i32 + 1);
            }
        }
        documents.push(Some(document));
        Ok(documents.len() as i32)
    }

    /// Close a document.
    pub fn close(&self, handle: i32) -> Result<()> {
        let index = Self::index(handle)?;
        let mut documents = self.documents.borrow_mut();
        let slot = documents
            .get_mut(index)
            .ok_or_else(|| Error::invalid(format!("handle {handle} is not open")))?;
        if slot.take().is_none() {
            return Err(Error::invalid(format!("handle {handle} is not open")));
        }
        Ok(())
    }

    /// Read a document.
    pub fn with<T>(&self, handle: i32, action: impl FnOnce(&Document) -> Result<T>) -> Result<T> {
        let documents = self.documents.borrow();
        let document = documents
            .get(Self::index(handle)?)
            .and_then(|slot| slot.as_ref())
            .ok_or_else(|| Error::invalid(format!("handle {handle} is not open")))?;
        action(document)
    }

    /// Read and change a document.
    pub fn with_mut<T>(
        &self,
        handle: i32,
        action: impl FnOnce(&mut Document) -> Result<T>,
    ) -> Result<T> {
        let mut documents = self.documents.borrow_mut();
        let document = documents
            .get_mut(Self::index(handle)?)
            .and_then(|slot| slot.as_mut())
            .ok_or_else(|| Error::invalid(format!("handle {handle} is not open")))?;
        action(document)
    }

    /// The number of documents currently open.
    pub fn open_count(&self) -> usize {
        self.documents
            .borrow()
            .iter()
            .filter(|slot| slot.is_some())
            .count()
    }

    /// The index a handle names.
    fn index(handle: i32) -> Result<usize> {
        if handle <= 0 {
            return Err(Error::invalid(format!("handle {handle} is not open")));
        }
        Ok(handle as usize - 1)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new()
    }
}

/// The rectangle a region covers, checked against the image.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn checked_rect(document: &Document, x: u32, y: u32, width: u32, height: u32) -> Result<Rect> {
    let rect = Rect::new(x, y, width, height)?;
    let meta = document.meta();
    if rect.right()? > meta.width() || rect.bottom()? > meta.height() {
        return Err(Error::invalid(format!(
            "the region {width}x{height}+{x}+{y} is outside the {}x{} image",
            meta.width(),
            meta.height()
        )));
    }
    Ok(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vpng_core::{CanonicalImage, PixelFormat};
    use vpng_decoder::Document;
    use vpng_encoder::EncodeOptions;

    /// A deterministic raw RGB8 image: 300 × 200.
    fn pixels() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(300 * 200 * 3);
        for y in 0..200u32 {
            for x in 0..300u32 {
                bytes.push((x % 251) as u8);
                bytes.push((y % 241) as u8);
                bytes.push((((x / 16) + (y / 16)) % 2 * 255) as u8);
            }
        }
        bytes
    }

    /// Encode the image and return the container's bytes.
    fn container() -> Vec<u8> {
        let image = CanonicalImage::from_vec(300, 200, PixelFormat::rgb8(), pixels())
            .expect("the image is canonical");
        let options = EncodeOptions::default().with_tile_size(64, 64);
        vpng_encoder::encode(&image, &options)
            .expect("the image encodes")
            .bytes
    }

    #[test]
    fn a_whole_container_opens_and_reports_its_layout() {
        let registry = Registry::new();
        let handle = registry
            .open_whole(container())
            .expect("the container opens");
        assert!(handle > 0, "a handle is never zero");

        registry
            .with(handle, |document| {
                assert_eq!(document.meta().width(), 300);
                assert_eq!(document.meta().height(), 200);
                assert_eq!(document.meta().format(), PixelFormat::rgb8());
                assert_eq!(document.meta().tile_count(), 20);
                assert_eq!(document.meta().level_count(), 1);
                Ok(())
            })
            .expect("the document is readable");
    }

    #[test]
    fn the_region_is_the_same_pixels_the_native_decoder_produces() {
        let bytes = container();
        let registry = Registry::new();
        let handle = registry
            .open_whole(bytes.clone())
            .expect("the container opens");

        let rect = Rect::new(64, 32, 128, 96).expect("the rectangle is valid");
        let native = Document::parse(&bytes, &Limits::default())
            .expect("the native decoder opens it")
            .decode_region(rect, &Limits::default())
            .expect("the native decoder decodes it");

        let wasm = registry
            .with_mut(handle, |document| {
                document.decode_region(rect, &Limits::default())
            })
            .expect("the module decodes it");

        assert_eq!(wasm, native.into_data(), "the pixels differ");
    }

    #[test]
    fn a_container_opened_from_its_parts_has_the_same_layout() {
        let bytes = container();
        let registry = Registry::new();
        let whole = registry
            .open_whole(bytes.clone())
            .expect("the container opens");
        // The directory is the entries plus the four-byte checksum trailer.
        let directory_size = vpng_container::Directory::total_size(20).expect("the size is known");
        let parts = registry
            .open_parts(
                &bytes[..64],
                &bytes[64..64 + directory_size],
                bytes.len() as u64,
            )
            .expect("the parts open");

        registry
            .with(whole, |document| {
                let expected = document.meta().tile_count();
                registry.with(parts, |other| {
                    assert_eq!(other.meta().tile_count(), expected);
                    assert_eq!(other.meta().width(), document.meta().width());
                    Ok(())
                })
            })
            .expect("both documents are readable");
    }

    #[test]
    fn a_payload_with_a_broken_checksum_is_refused() {
        let bytes = container();
        let registry = Registry::new();
        let handle = registry
            .open_whole(bytes.clone())
            .expect("the container opens");

        // The first tile's payload, with one byte flipped.
        let entry = registry
            .with(handle, |document| {
                Ok(document
                    .meta()
                    .tile_entry(0)
                    .copied()
                    .expect("there is a tile"))
            })
            .expect("the entry is readable");
        let start = entry.payload_offset as usize;
        let end = start + entry.compressed_length as usize;
        let mut payload = bytes[start..end].to_vec();
        let last = payload.len() - 1;
        payload[last] ^= 0xFF;

        let error = registry
            .with(handle, |document| {
                document.decode_payload(0, &payload, &Limits::default())
            })
            .expect_err("a broken payload is refused");
        assert!(
            error.to_string().contains("checksum mismatch"),
            "the error does not name the checksum: {error}"
        );
    }

    #[test]
    fn a_payload_of_the_wrong_length_is_refused() {
        let bytes = container();
        let registry = Registry::new();
        let handle = registry.open_whole(bytes).expect("the container opens");

        let error = registry
            .with(handle, |document| {
                document.decode_payload(0, &[0u8; 3], &Limits::default())
            })
            .expect_err("a short payload is refused");
        assert!(
            error.to_string().contains("the directory says"),
            "the error does not name the directory: {error}"
        );
    }

    #[test]
    fn a_closed_handle_is_refused() {
        let registry = Registry::new();
        let handle = registry
            .open_whole(container())
            .expect("the container opens");
        registry.close(handle).expect("the document closes");
        assert!(
            registry.with(handle, |_| Ok(())).is_err(),
            "a closed handle was accepted"
        );
        assert!(
            registry.close(handle).is_err(),
            "closing twice was accepted"
        );
        assert!(
            registry.with(0, |_| Ok(())).is_err(),
            "handle zero was accepted"
        );
    }

    #[test]
    fn a_region_outside_the_image_is_refused() {
        let registry = Registry::new();
        let handle = registry
            .open_whole(container())
            .expect("the container opens");
        let error = registry
            .with(handle, |document| checked_rect(document, 290, 190, 32, 32))
            .expect_err("a region outside the image is refused");
        assert!(
            error.to_string().contains("outside"),
            "the error does not say the region is outside: {error}"
        );
    }

    #[test]
    fn the_canonical_image_is_what_the_format_stores() {
        // The bytes the module returns are canonical: rows tightly packed.
        let bytes = container();
        let registry = Registry::new();
        let handle = registry.open_whole(bytes).expect("the container opens");
        let rect = Rect::new(0, 0, 10, 10).expect("the rectangle is valid");
        let decoded = registry
            .with_mut(handle, |document| {
                document.decode_region(rect, &Limits::default())
            })
            .expect("the module decodes it");
        assert_eq!(decoded.len(), 10 * 10 * 3, "the region is the wrong size");

        let image = CanonicalImage::from_vec_checked(
            10,
            10,
            PixelFormat::rgb8(),
            decoded,
            &Limits::default(),
        )
        .expect("the bytes are canonical");
        assert_eq!(image.width(), 10);
    }

    /// A layered container: a base, an overlay and a mask.
    fn layered_container() -> Vec<u8> {
        use vpng_core::Rect;
        use vpng_encoder::{encode_layers, EncodeOptions, LayerInput};

        let base = CanonicalImage::from_vec(64, 48, PixelFormat::rgba8(), {
            let mut bytes = Vec::with_capacity(64 * 48 * 4);
            for index in 0..64 * 48 {
                bytes.extend_from_slice(&[
                    (index % 251) as u8,
                    (index % 97) as u8,
                    (index % 31) as u8,
                    0xFF,
                ]);
            }
            bytes
        })
        .expect("the base is canonical");
        let overlay = CanonicalImage::from_vec(
            32,
            16,
            PixelFormat::rgba8(),
            [0x20, 0x30, 0x40, 0xFF].repeat(32 * 16),
        )
        .expect("the overlay is canonical");
        let mask = CanonicalImage::from_vec(
            32,
            16,
            PixelFormat::rgba8(),
            [0x10, 0x20, 0x30, 0x80].repeat(32 * 16),
        )
        .expect("the mask is canonical");

        let layers = [
            LayerInput::pixels("base", Rect::new(0, 0, 64, 48).unwrap(), &base),
            LayerInput::pixels("ink", Rect::new(8, 8, 32, 16).unwrap(), &overlay).with_mask(2),
            LayerInput::mask("m", Rect::new(8, 8, 32, 16).unwrap(), &mask),
        ];
        encode_layers(&layers, &EncodeOptions::default().with_tile_size(64, 64))
            .expect("the layers encode")
            .bytes
    }

    #[test]
    fn a_layered_container_reports_its_layers() {
        let registry = Registry::new();
        let handle = registry
            .open_whole(layered_container())
            .expect("the container opens");

        registry
            .with(handle, |document| {
                assert_eq!(document.meta().layer_count(), 3);
                let base = document.meta().layer(0).expect("the base layer");
                assert_eq!(base.name(), "base");
                assert!(!base.is_mask());
                let mask = document.meta().layer(2).expect("the mask layer");
                assert_eq!(mask.name(), "m");
                assert!(mask.is_mask());
                Ok(())
            })
            .expect("the document is readable");
    }

    #[test]
    fn a_layer_decodes_to_the_same_pixels_the_native_decoder_produces() {
        let bytes = layered_container();
        let registry = Registry::new();
        let handle = registry
            .open_whole(bytes.clone())
            .expect("the container opens");

        let native = Document::parse(&bytes, &Limits::default())
            .expect("the native decoder opens it")
            .decode_layer(1, &Limits::default())
            .expect("the native decoder decodes it");

        let wasm = registry
            .with_mut(handle, |document| {
                document.decode_layer(1, &Limits::default())
            })
            .expect("the module decodes it");

        assert_eq!(wasm, native.into_data(), "the pixels differ");
    }
}
