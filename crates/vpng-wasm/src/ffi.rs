//! The exported WebAssembly functions.
//!
//! This is the module's boundary with JavaScript. It is a **raw `extern "C"`
//! ABI**, deliberately: there is no `wasm-bindgen`, no generated glue and no
//! build-time code generation. A browser can instantiate the `.wasm` file
//! with `WebAssembly.instantiate` and call these functions directly, and so
//! can Node, and so can any other WebAssembly runtime.
//!
//! # The conventions
//!
//! * **Handles** are `i32`. A handle is an index into the registry plus one,
//!   so zero is always an error and never a document.
//! * **Return values** come in three shapes:
//!   * a **status** is `i32`: zero is success and a negative value is the
//!     negative of the error's exit code — **the same codes the command line
//!     uses**, so `-4` is a corrupt file and `-3` is an unsupported feature;
//!   * a **count** or a **small value** is `i32`, with the same convention;
//!   * a **byte length or a byte offset** is `i64`, because a 16K RGBA16
//!     region is 2 GiB and a file may be larger than that.
//! * **Errors** are described by a message the caller fetches with
//!   [`vpng_error_length`] and [`vpng_error_copy`].
//! * **Pointers** are `u32` offsets into the module's linear memory. Every
//!   pointer is checked against the memory's real size before it is used, so
//!   a bad pointer from JavaScript is an error and not undefined behaviour.
//!
//! # Memory
//!
//! The module allocates with the WebAssembly allocator, which grows the
//! linear memory as it needs to. [`vpng_alloc`] hands out a buffer and
//! [`vpng_dealloc`] returns it. A caller that writes into a buffer must
//! remember that the memory may **grow** when the module allocates, which
//! invalidates any JavaScript view it had cached — the wrapper in
//! `wasm/vpng.js` re-creates its views after every call that can allocate.
//!
//! # Threading
//!
//! A WebAssembly instance is single-threaded here. The registry and the last
//! error message are held behind a mutex anyway, so the module's state is
//! well-defined even if a future runtime instantiates it on more than one
//! thread.

use std::alloc::{alloc, dealloc, Layout};
use std::sync::{Mutex, MutexGuard};

use vpng_core::{Error, ErrorKind, Limits, Result};

use crate::registry::{checked_rect, Document, Registry};

/// The open documents.
static REGISTRY: Mutex<Registry> = Mutex::new(Registry::new());

/// The last error's message.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

/// The number of bytes in a WebAssembly page.
const PAGE_SIZE: usize = 65_536;

/// The registry, with a poisoned lock recovered rather than propagated.
fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The last error's message, with a poisoned lock recovered.
fn last_error() -> MutexGuard<'static, String> {
    LAST_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The size of the module's linear memory, in bytes.
fn memory_size() -> usize {
    // `memory_size` is the WebAssembly `memory.size` instruction: it counts
    // **pages**, and a page is 64 KiB.
    core::arch::wasm32::memory_size(0) * PAGE_SIZE
}

/// Read a slice of the module's memory.
///
/// The bounds are checked against the memory's actual size, so a pointer
/// that JavaScript made up is an error rather than undefined behaviour.
fn read_slice(ptr: u32, len: u32) -> Result<&'static [u8]> {
    if len == 0 {
        // An empty range is always in range, and a null pointer with a zero
        // length is not something `from_raw_parts` may be given.
        return Ok(&[]);
    }
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or_else(|| Error::invalid(format!("the range {ptr}..{len} overflows")))?;
    let total = memory_size();
    if end > total {
        return Err(Error::invalid(format!(
            "the range {ptr}..{end} is outside the module's {total} bytes of memory"
        )));
    }
    // SAFETY: the range was just checked against the memory's size, and the
    // memory of a WebAssembly module is not moved while the caller holds
    // this slice: the module is single-threaded and nothing in this call
    // allocates.
    Ok(unsafe { std::slice::from_raw_parts(start as *const u8, len as usize) })
}

/// Write into a slice of the module's memory.
fn write_slice(ptr: u32, len: u32) -> Result<&'static mut [u8]> {
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or_else(|| Error::invalid(format!("the range {ptr}..{len} overflows")))?;
    let total = memory_size();
    if end > total {
        return Err(Error::invalid(format!(
            "the range {ptr}..{end} is outside the module's {total} bytes of memory"
        )));
    }
    // SAFETY: the range was just checked against the memory's size, and the
    // memory is not moved while the caller holds this slice.
    Ok(unsafe { std::slice::from_raw_parts_mut(start as *mut u8, len as usize) })
}

/// A return type the module's conventions can express.
///
/// The error code is negative, so a type that has no room for one reports
/// failure as its zero value.
trait FfiValue: Copy {
    /// The value that stands for the error code `code`.
    fn from_code(code: i32) -> Self;
}

impl FfiValue for i32 {
    fn from_code(code: i32) -> Self {
        code
    }
}

impl FfiValue for i64 {
    fn from_code(code: i32) -> Self {
        code as i64
    }
}

/// Record an error and return its code.
fn fail(error: Error) -> i32 {
    let message = error.to_string();
    let code = error.exit_code();
    *last_error() = message;
    if code > 0 {
        -code
    } else {
        -1
    }
}

/// Run an action and turn its result into the module's return convention.
fn guard<T: FfiValue>(action: impl FnOnce() -> Result<T>) -> T {
    match action() {
        Ok(value) => value,
        Err(error) => T::from_code(fail(error)),
    }
}

/// Run an action and return the error's code, or zero on success.
fn status(action: impl FnOnce() -> Result<()>) -> i32 {
    match action() {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// Allocate `len` bytes in the module's heap.
///
/// The pointer is zero on failure. The caller must return the buffer with
/// [`vpng_dealloc`], passing the same length.
#[no_mangle]
pub extern "C" fn vpng_alloc(len: u32) -> u32 {
    let Ok(layout) = Layout::from_size_align(len as usize, 1) else {
        return 0;
    };
    if layout.size() == 0 {
        return 0;
    }
    // SAFETY: the layout has a non-zero size, so `alloc` is called with a
    // valid layout. The returned block is owned by the caller until
    // `vpng_dealloc` is called with the same layout.
    let block = unsafe { alloc(layout) };
    if block.is_null() {
        return 0;
    }
    block as u32
}

/// Return a buffer to the module's heap.
///
/// `len` must be the length the buffer was allocated with.
#[no_mangle]
pub extern "C" fn vpng_dealloc(ptr: u32, len: u32) {
    let Ok(layout) = Layout::from_size_align(len as usize, 1) else {
        return;
    };
    if layout.size() == 0 || ptr == 0 {
        return;
    }
    // SAFETY: the block was allocated by `vpng_alloc` with this layout and
    // has not been freed, because the caller holds it until it calls this.
    unsafe { dealloc(ptr as *mut u8, layout) }
}

/// Open a whole container held in the module's memory.
///
/// Returns a handle, or a negative error code.
#[no_mangle]
pub extern "C" fn vpng_open(data_ptr: u32, len: u32) -> i32 {
    let bytes = match read_slice(data_ptr, len) {
        Ok(bytes) => bytes.to_vec(),
        Err(error) => return fail(error),
    };
    let registry = registry();
    match registry.open_whole(bytes) {
        Ok(handle) => handle,
        Err(error) => fail(error),
    }
}

/// Open a container from its header and its directory alone.
///
/// The caller supplies the two byte ranges it fetched; the module then
/// knows the container's layout and can be asked for the payloads it wants.
/// `file_size` is the size the container claims for itself.
///
/// A container with a layer table needs the table as well, so this refuses
/// one: use `vpng_open_parts_with_layers`.
#[no_mangle]
pub extern "C" fn vpng_open_parts(
    header_ptr: u32,
    header_len: u32,
    directory_ptr: u32,
    directory_len: u32,
    file_size: u32,
) -> i32 {
    vpng_open_parts_with_layers(
        header_ptr,
        header_len,
        directory_ptr,
        directory_len,
        0,
        0,
        file_size,
    )
}

/// Open a container from its header, its directory and its layer table.
///
/// The layer table's pointer and length may both be zero, which means the
/// container has none. A container that *declares* a table and is given none
/// is refused: the table says what the container means.
#[no_mangle]
pub extern "C" fn vpng_open_parts_with_layers(
    header_ptr: u32,
    header_len: u32,
    directory_ptr: u32,
    directory_len: u32,
    layer_table_ptr: u32,
    layer_table_len: u32,
    file_size: u32,
) -> i32 {
    vpng_open_parts_with_tables(
        header_ptr,
        header_len,
        directory_ptr,
        directory_len,
        layer_table_ptr,
        layer_table_len,
        0,
        0,
        file_size,
    )
}

/// Open a container from its header, its directory and its tables.
///
/// The layer table's and the GPU table's pointers and lengths may all be
/// zero, which means the container has none of them. A container that
/// *declares* a table and is given none is refused: the table says what the
/// container means.
#[no_mangle]
pub extern "C" fn vpng_open_parts_with_tables(
    header_ptr: u32,
    header_len: u32,
    directory_ptr: u32,
    directory_len: u32,
    layer_table_ptr: u32,
    layer_table_len: u32,
    gpu_table_ptr: u32,
    gpu_table_len: u32,
    file_size: u32,
) -> i32 {
    let header = match read_slice(header_ptr, header_len) {
        Ok(bytes) => bytes.to_vec(),
        Err(error) => return fail(error),
    };
    let directory = match read_slice(directory_ptr, directory_len) {
        Ok(bytes) => bytes.to_vec(),
        Err(error) => return fail(error),
    };
    let layer_table = if layer_table_len == 0 {
        None
    } else {
        match read_slice(layer_table_ptr, layer_table_len) {
            Ok(bytes) => Some(bytes.to_vec()),
            Err(error) => return fail(error),
        }
    };
    let gpu_table = if gpu_table_len == 0 {
        None
    } else {
        match read_slice(gpu_table_ptr, gpu_table_len) {
            Ok(bytes) => Some(bytes.to_vec()),
            Err(error) => return fail(error),
        }
    };
    let registry = registry();
    match registry.open_parts_with_tables(
        &header,
        &directory,
        layer_table.as_deref(),
        gpu_table.as_deref(),
        file_size as u64,
    ) {
        Ok(handle) => handle,
        Err(error) => fail(error),
    }
}

/// Close a document.
#[no_mangle]
pub extern "C" fn vpng_close(handle: i32) -> i32 {
    let registry = registry();
    status(|| registry.close(handle))
}

/// The image's width in pixels.
#[no_mangle]
pub extern "C" fn vpng_width(handle: i32) -> i32 {
    let registry = registry();
    guard(|| registry.with(handle, |document| Ok(document.meta().width() as i32)))
}

/// The image's height in pixels.
#[no_mangle]
pub extern "C" fn vpng_height(handle: i32) -> i32 {
    let registry = registry();
    guard(|| registry.with(handle, |document| Ok(document.meta().height() as i32)))
}

/// The pixel format's code.
///
/// The codes are the ones the Python binding's `FORMATS` list uses, in the
/// same order: `gray8`, `gray16`, `graya8`, `graya16`, `rgb8`, `rgb16`,
/// `rgba8`, `rgba16`.
#[no_mangle]
pub extern "C" fn vpng_format(handle: i32) -> i32 {
    let registry = registry();
    guard(|| registry.with(handle, |document| Ok(format_code(document.meta().format()))))
}

/// The number of bytes one pixel takes.
#[no_mangle]
pub extern "C" fn vpng_bytes_per_pixel(handle: i32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            Ok(document.meta().format().bytes_per_pixel() as i32)
        })
    })
}

/// The number of tiles in the container.
#[no_mangle]
pub extern "C" fn vpng_tile_count(handle: i32) -> i32 {
    let registry = registry();
    guard(|| registry.with(handle, |document| Ok(document.meta().tile_count() as i32)))
}

/// The number of levels in the container.
#[no_mangle]
pub extern "C" fn vpng_level_count(handle: i32) -> i32 {
    let registry = registry();
    guard(|| registry.with(handle, |document| Ok(document.meta().level_count() as i32)))
}

/// The number of layers in the container.
///
/// A container written before v0.4 has exactly one layer: the base layer.
#[no_mangle]
pub extern "C" fn vpng_layer_count(handle: i32) -> i32 {
    let registry = registry();
    guard(|| registry.with(handle, |document| Ok(document.meta().layer_count() as i32)))
}

/// The length of a layer's name, in bytes.
///
/// A layer's name may be empty. The name itself is fetched with
/// [`vpng_layer_name_copy`].
#[no_mangle]
pub extern "C" fn vpng_layer_name_length(handle: i32, layer: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let name = document
                .meta()
                .layer(layer as usize)
                .ok_or_else(|| Error::invalid(format!("layer {layer} is out of range")))?;
            Ok(name.name().len() as i32)
        })
    })
}

/// Copy a layer's name into the caller's buffer.
///
/// Returns the number of bytes written. The name is truncated if the buffer
/// is too small.
#[no_mangle]
pub extern "C" fn vpng_layer_name_copy(handle: i32, layer: u32, out_ptr: u32, out_len: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let name = document
                .meta()
                .layer(layer as usize)
                .ok_or_else(|| Error::invalid(format!("layer {layer} is out of range")))?;
            let bytes = name.name().as_bytes();
            let take = bytes.len().min(out_len as usize);
            let out = write_slice(out_ptr, take as u32)?;
            out.copy_from_slice(&bytes[..take]);
            Ok(take as i32)
        })
    })
}

/// The rectangle a layer covers on the canvas.
///
/// The four values — `x`, `y`, `width`, `height` — are written into the
/// caller's buffer as little-endian `u32`s. Returns 0, or a negative error
/// code.
#[no_mangle]
pub extern "C" fn vpng_layer_rect(handle: i32, layer: u32, out_ptr: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let layer = document
                .meta()
                .layer(layer as usize)
                .ok_or_else(|| Error::invalid(format!("layer {layer} is out of range")))?;
            let rect = layer.rect();
            let out = write_slice(out_ptr, 16)?;
            out[0..4].copy_from_slice(&rect.x.to_le_bytes());
            out[4..8].copy_from_slice(&rect.y.to_le_bytes());
            out[8..12].copy_from_slice(&rect.width.to_le_bytes());
            out[12..16].copy_from_slice(&rect.height.to_le_bytes());
            Ok(0)
        })
    })
}

/// Whether a layer is a mask rather than picture pixels.
///
/// Returns 1 for a mask, 0 for picture pixels, or a negative error code.
#[no_mangle]
pub extern "C" fn vpng_layer_is_mask(handle: i32, layer: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let layer = document
                .meta()
                .layer(layer as usize)
                .ok_or_else(|| Error::invalid(format!("layer {layer} is out of range")))?;
            Ok(i32::from(layer.is_mask()))
        })
    })
}

/// The number of canonical bytes one layer's pixels take, or a negative error
/// code.
///
/// The layer's pixels are sized to the layer's own rectangle, not to the
/// canvas.
#[no_mangle]
pub extern "C" fn vpng_layer_length(handle: i32, layer: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let layer = document
                .meta()
                .layer(layer as usize)
                .ok_or_else(|| Error::invalid(format!("layer {layer} is out of range")))?;
            let rect = layer.rect();
            let bytes = document.meta().format().bytes_per_pixel();
            Ok(rect.width as i64 * rect.height as i64 * bytes as i64)
        })
    })
}

/// Decode one layer's own pixels into the caller's buffer.
///
/// Returns the number of bytes written, or a negative error code. The buffer
/// must be at least [`vpng_layer_length`] bytes long.
#[no_mangle]
pub extern "C" fn vpng_decode_layer(handle: i32, layer: u32, out_ptr: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with_mut(handle, |document| {
            let pixels = document.decode_layer(layer as usize, &Limits::default())?;
            let out = write_slice(out_ptr, pixels.len() as u32)?;
            out.copy_from_slice(&pixels);
            Ok(pixels.len() as i64)
        })
    })
}

/// The byte offset of a tile's payload, or a negative error code.
///
/// A caller that reads over HTTP uses this to build its own range request.
#[no_mangle]
pub extern "C" fn vpng_tile_offset(handle: i32, index: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let entry = document
                .meta()
                .tile_entry(index as usize)
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(entry.payload_offset as i64)
        })
    })
}

/// The compressed length of a tile's payload.
#[no_mangle]
pub extern "C" fn vpng_tile_length(handle: i32, index: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let entry = document
                .meta()
                .tile_entry(index as usize)
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(entry.compressed_length as i64)
        })
    })
}

/// The uncompressed length of a tile.
#[no_mangle]
pub extern "C" fn vpng_tile_uncompressed_length(handle: i32, index: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let entry = document
                .meta()
                .tile_entry(index as usize)
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(entry.uncompressed_length as i64)
        })
    })
}

/// The codec id a tile's payload uses.
#[no_mangle]
pub extern "C" fn vpng_tile_codec(handle: i32, index: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let entry = document
                .meta()
                .tile_entry(index as usize)
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(entry.codec_id as i32)
        })
    })
}

/// The level a tile belongs to.
#[no_mangle]
pub extern "C" fn vpng_tile_level(handle: i32, index: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let entry = document
                .meta()
                .tile_entry(index as usize)
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok((entry.flags as i32) & 0x00FF)
        })
    })
}

/// The tiles of the original level that a region touches.
///
/// The indices are written into the caller's buffer, one `u32` each, and the
/// number written is returned. The caller supplies the buffer's capacity; if
/// it is too small the call fails and says how many are needed.
///
/// This is what makes the **range** path possible: a browser that has read
/// only the header and the directory asks this which tiles it needs, fetches
/// exactly those payloads, and hands them back one at a time.
#[no_mangle]
pub extern "C" fn vpng_region_tiles(
    handle: i32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    out_ptr: u32,
    capacity: u32,
) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let rect = checked_rect(document, x, y, width, height)?;
            let level = document.meta().base_level();
            let positions = level.partition().tiles_for(rect)?;
            if positions.len() > capacity as usize {
                return Err(Error::invalid(format!(
                    "the region touches {} tiles, but the buffer holds {}",
                    positions.len(),
                    capacity
                )));
            }
            let out = write_slice(out_ptr, (positions.len() * 4) as u32)?;
            for (slot, position) in positions.iter().enumerate() {
                let entry = level.entry_index(*position).ok_or_else(|| {
                    Error::invalid(format!("the tile at position {position} is out of range"))
                })?;
                out[slot * 4..slot * 4 + 4].copy_from_slice(&(entry as u32).to_le_bytes());
            }
            Ok(positions.len() as i32)
        })
    })
}

/// The rectangle a tile covers.
///
/// The four values — `x`, `y`, `width`, `height` — are written into the
/// caller's buffer as little-endian `u32`s. Returns 0, or a negative error
/// code.
#[no_mangle]
pub extern "C" fn vpng_tile_rect(handle: i32, index: u32, out_ptr: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let entry = document
                .meta()
                .tile_entry(index as usize)
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            let out = write_slice(out_ptr, 16)?;
            out[0..4].copy_from_slice(&entry.x.to_le_bytes());
            out[4..8].copy_from_slice(&entry.y.to_le_bytes());
            out[8..12].copy_from_slice(&(entry.width as u32).to_le_bytes());
            out[12..16].copy_from_slice(&(entry.height as u32).to_le_bytes());
            Ok(0)
        })
    })
}

/// The number of canonical bytes a region takes, or a negative error code.
#[no_mangle]
pub extern "C" fn vpng_region_length(handle: i32, x: u32, y: u32, width: u32, height: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let rect = checked_rect(document, x, y, width, height)?;
            let bytes = document.meta().format().bytes_per_pixel();
            Ok(rect.width as i64 * rect.height as i64 * bytes as i64)
        })
    })
}

/// Decode one rectangle into the caller's buffer.
///
/// Returns the number of bytes written, or a negative error code. The buffer
/// must be at least [`vpng_region_length`] bytes long.
#[no_mangle]
pub extern "C" fn vpng_decode_region(
    handle: i32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    out_ptr: u32,
) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with_mut(handle, |document| {
            let rect = checked_rect(document, x, y, width, height)?;
            let pixels = document.decode_region(rect, &Limits::default())?;
            let out = write_slice(out_ptr, pixels.len() as u32)?;
            out.copy_from_slice(&pixels);
            Ok(pixels.len() as i64)
        })
    })
}

/// Decode one tile payload the caller has fetched into the caller's buffer.
///
/// The payload's checksum is verified against the directory before the codec
/// sees it. Returns the number of bytes written, or a negative error code.
/// The buffer must be at least [`vpng_tile_uncompressed_length`] bytes long.
#[no_mangle]
pub extern "C" fn vpng_decode_payload(
    handle: i32,
    index: u32,
    payload_ptr: u32,
    payload_len: u32,
    out_ptr: u32,
) -> i64 {
    let payload = match read_slice(payload_ptr, payload_len) {
        Ok(bytes) => bytes.to_vec(),
        Err(error) => return fail(error) as i64,
    };
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let pixels = document.decode_payload(index as usize, &payload, &Limits::default())?;
            let out = write_slice(out_ptr, pixels.len() as u32)?;
            out.copy_from_slice(&pixels);
            Ok(pixels.len() as i64)
        })
    })
}

/// Verify every tile payload's checksum.
///
/// Returns 0 when every payload's CRC32C matches the directory, or a
/// negative error code naming the first tile that does not.
#[no_mangle]
pub extern "C" fn vpng_check(handle: i32) -> i32 {
    let registry = registry();
    status(|| {
        registry.with_mut(handle, |document| {
            let Document::Whole(reader) = document else {
                return Err(Error::unsupported(
                    "this document holds only the header and the directory, so there are no \
                     payloads to check",
                ));
            };
            let count = reader.meta().tile_count() as usize;
            for index in 0..count {
                reader.decode_tile(index, &Limits::default())?;
            }
            Ok(())
        })
    })
}

/// Whether the container carries a GPU payload table.
///
/// Returns 1 when it does, 0 when it does not, or a negative error code.
///
/// The GPU payload is the file's optional, explicitly named, **lossy**
/// sibling of a tile's lossless payload. It is never used to reconstruct the
/// image.
#[no_mangle]
pub extern "C" fn vpng_gpu_has_table(handle: i32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            Ok(i32::from(document.meta().gpu_table().is_some()))
        })
    })
}

/// The number of records in the container's GPU payload table.
///
/// One record per tile, in the directory's canonical order. A container with
/// no GPU table has zero records.
#[no_mangle]
pub extern "C" fn vpng_gpu_record_count(handle: i32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            Ok(document.meta().gpu_table().map_or(0, |table| table.len()) as i32)
        })
    })
}

/// The number of bytes every GPU payload in the container occupies.
#[no_mangle]
pub extern "C" fn vpng_gpu_payload_bytes(handle: i32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            Ok(document
                .meta()
                .gpu_table()
                .map_or(0, |table| table.total_payload_bytes()) as i64)
        })
    })
}

/// The byte offset of a tile's GPU payload, or a negative error code.
///
/// A caller that reads over HTTP uses this to build its own range request.
#[no_mangle]
pub extern "C" fn vpng_gpu_payload_offset(handle: i32, index: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let record = document
                .meta()
                .gpu_table()
                .and_then(|table| table.get(index as usize))
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(record.payload_offset as i64)
        })
    })
}

/// The length of a tile's GPU payload, or a negative error code.
#[no_mangle]
pub extern "C" fn vpng_gpu_payload_length(handle: i32, index: u32) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let record = document
                .meta()
                .gpu_table()
                .and_then(|table| table.get(index as usize))
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(i64::from(record.payload_length))
        })
    })
}

/// The colour codec a tile's GPU payload uses: 0 for none, 7 for BC1.
#[no_mangle]
pub extern "C" fn vpng_gpu_colour_codec(handle: i32, index: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let record = document
                .meta()
                .gpu_table()
                .and_then(|table| table.get(index as usize))
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(i32::from(record.colour_codec))
        })
    })
}

/// The alpha codec a tile's GPU payload uses: 0 for none, 8 for BC4.
#[no_mangle]
pub extern "C" fn vpng_gpu_alpha_codec(handle: i32, index: u32) -> i32 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let record = document
                .meta()
                .gpu_table()
                .and_then(|table| table.get(index as usize))
                .ok_or_else(|| Error::invalid(format!("tile index {index} is out of range")))?;
            Ok(i32::from(record.alpha_codec))
        })
    })
}

/// The number of canonical bytes a GPU region takes, or a negative error code.
///
/// The result is always **RGBA8**: a GPU payload is always eight-bit, and a
/// texture is always RGBA.
#[no_mangle]
pub extern "C" fn vpng_gpu_region_length(
    handle: i32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let rect = checked_rect(document, x, y, width, height)?;
            Ok(rect.width as i64 * rect.height as i64 * 4)
        })
    })
}

/// Decode one rectangle of the container's **GPU payload** into the caller's
/// buffer.
///
/// The result is **lossy** — BC1 and BC4 throw information away — and it is
/// always **RGBA8**. The exact pixels are [`vpng_decode_region`], and nothing
/// exact reads this path.
///
/// Returns the number of bytes written, or a negative error code. The buffer
/// must be at least [`vpng_gpu_region_length`] bytes long.
#[no_mangle]
pub extern "C" fn vpng_decode_gpu_region(
    handle: i32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    out_ptr: u32,
) -> i64 {
    let registry = registry();
    guard(|| {
        registry.with_mut(handle, |document| {
            let rect = checked_rect(document, x, y, width, height)?;
            let pixels = document.decode_gpu_region(rect, &Limits::default())?;
            let out = write_slice(out_ptr, pixels.len() as u32)?;
            out.copy_from_slice(&pixels);
            Ok(pixels.len() as i64)
        })
    })
}

/// Decode one tile's **GPU payload**, which the caller has fetched, into the
/// caller's buffer.
///
/// The payload's checksum is verified against the GPU table before the block
/// decoder sees it. The result is **lossy** and is always RGBA8.
///
/// Returns the number of bytes written, or a negative error code.
#[no_mangle]
pub extern "C" fn vpng_decode_gpu_payload(
    handle: i32,
    index: u32,
    payload_ptr: u32,
    payload_len: u32,
    out_ptr: u32,
) -> i64 {
    let payload = match read_slice(payload_ptr, payload_len) {
        Ok(bytes) => bytes.to_vec(),
        Err(error) => return fail(error) as i64,
    };
    let registry = registry();
    guard(|| {
        registry.with(handle, |document| {
            let pixels =
                document.decode_gpu_payload(index as usize, &payload, &Limits::default())?;
            let out = write_slice(out_ptr, pixels.len() as u32)?;
            out.copy_from_slice(&pixels);
            Ok(pixels.len() as i64)
        })
    })
}

/// The length of the last error's message, in bytes.
#[no_mangle]
pub extern "C" fn vpng_error_length() -> i32 {
    last_error().len() as i32
}

/// Copy the last error's message into the caller's buffer.
///
/// Returns the number of bytes written. The message is truncated if the
/// buffer is too small.
#[no_mangle]
pub extern "C" fn vpng_error_copy(out_ptr: u32, out_len: u32) -> i32 {
    let message = last_error().clone();
    let bytes = message.as_bytes();
    let take = bytes.len().min(out_len as usize);
    match write_slice(out_ptr, take as u32) {
        Ok(out) => {
            out.copy_from_slice(&bytes[..take]);
            take as i32
        }
        Err(error) => fail(error),
    }
}

/// The SHA-256 of a range of the module's memory.
///
/// The digest is written into the caller's buffer, which must be 32 bytes
/// long. Returns 0, or a negative error code.
#[no_mangle]
pub extern "C" fn vpng_sha256(data_ptr: u32, len: u32, out_ptr: u32) -> i32 {
    let data = match read_slice(data_ptr, len) {
        Ok(bytes) => bytes,
        Err(error) => return fail(error),
    };
    let digest = vpng_container::sha256(data);
    match write_slice(out_ptr, 32) {
        Ok(out) => {
            out.copy_from_slice(&digest);
            0
        }
        Err(error) => fail(error),
    }
}

/// The version of the container format this module understands.
///
/// The major version is in the high half and the minor version in the low
/// half, so the value is `0x00030003` for format v0.3.
#[no_mangle]
pub extern "C" fn vpng_format_version() -> i32 {
    ((vpng_container::FORMAT_MAJOR as i32) << 16) | vpng_container::FORMAT_MINOR as i32
}

/// The canonical digest of a region the caller has decoded.
///
/// The digest is the one the specification defines and the one the command
/// line prints: a domain-separation prefix, the width, the height, the
/// color-model id, the bits-per-channel value and the packed pixels, hashed
/// together. It is computed by the **same** Rust function the command line
/// uses, so the two cannot disagree about what "the same pixels" means.
///
/// The digest is written into the caller's buffer, which must be 32 bytes
/// long. Returns 0, or a negative error code.
#[no_mangle]
pub extern "C" fn vpng_canonical_digest(
    data_ptr: u32,
    data_len: u32,
    width: u32,
    height: u32,
    format: u32,
    out_ptr: u32,
) -> i32 {
    let data = match read_slice(data_ptr, data_len) {
        Ok(bytes) => bytes,
        Err(error) => return fail(error),
    };
    let pixel_format = match format_from_code(format) {
        Ok(format) => format,
        Err(error) => return fail(error),
    };
    let image = match vpng_core::CanonicalImage::from_vec_checked(
        width,
        height,
        pixel_format,
        data.to_vec(),
        &Limits::default(),
    ) {
        Ok(image) => image,
        Err(error) => return fail(error),
    };
    let digest = vpng_container::canonical_digest(&image);
    match write_slice(out_ptr, 32) {
        Ok(out) => {
            out.copy_from_slice(&digest);
            0
        }
        Err(error) => fail(error),
    }
}

/// The pixel format's numeric code.
///
/// The codes are the ones the Python binding's `FORMATS` list uses, in the
/// same order.
fn format_code(format: vpng_core::PixelFormat) -> i32 {
    use vpng_core::{BitsPerChannel, ColorModel};
    match (format.color_model, format.bits) {
        (ColorModel::Gray, BitsPerChannel::Eight) => 0,
        (ColorModel::Gray, BitsPerChannel::Sixteen) => 1,
        (ColorModel::GrayAlpha, BitsPerChannel::Eight) => 2,
        (ColorModel::GrayAlpha, BitsPerChannel::Sixteen) => 3,
        (ColorModel::Rgb, BitsPerChannel::Eight) => 4,
        (ColorModel::Rgb, BitsPerChannel::Sixteen) => 5,
        (ColorModel::Rgba, BitsPerChannel::Eight) => 6,
        (ColorModel::Rgba, BitsPerChannel::Sixteen) => 7,
        (ColorModel::Indexed, _) => 8,
    }
}

/// The pixel format a code names.
///
/// The codes are the ones the Python binding's `FORMATS` list uses, in the
/// same order.
fn format_from_code(code: u32) -> Result<vpng_core::PixelFormat> {
    use vpng_core::PixelFormat;
    Ok(match code {
        0 => PixelFormat::gray8(),
        1 => PixelFormat::gray16(),
        2 => PixelFormat::gray_alpha8(),
        3 => PixelFormat::gray_alpha16(),
        4 => PixelFormat::rgb8(),
        5 => PixelFormat::rgb16(),
        6 => PixelFormat::rgba8(),
        7 => PixelFormat::rgba16(),
        other => {
            return Err(Error::invalid(format!(
                "there is no pixel format with the code {other}"
            )))
        }
    })
}

/// Report an error from a function that has no other way to fail.
///
/// The exported functions above use `fail` directly; this one exists so that
/// a caller can report an error it found itself.
#[no_mangle]
pub extern "C" fn vpng_report_error(code: i32, message_ptr: u32, message_len: u32) -> i32 {
    let message = match read_slice(message_ptr, message_len) {
        Ok(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Err(error) => return fail(error),
    };
    let kind = match -code {
        2 => ErrorKind::InvalidArgument,
        3 => ErrorKind::UnsupportedFeature,
        4 => ErrorKind::CorruptFile,
        5 => ErrorKind::ResourceLimit,
        6 => ErrorKind::SizeLimitExceeded,
        7 => ErrorKind::VerificationFailed,
        8 => ErrorKind::Io,
        _ => ErrorKind::Internal,
    };
    let error = match kind {
        ErrorKind::InvalidArgument => Error::invalid(message),
        ErrorKind::UnsupportedFeature => Error::unsupported(message),
        ErrorKind::CorruptFile => Error::corrupt(message),
        ErrorKind::ResourceLimit => Error::resource_limit(message),
        ErrorKind::VerificationFailed => Error::verification(message),
        ErrorKind::Io => Error::Io(std::io::Error::other(message)),
        ErrorKind::SizeLimitExceeded => Error::internal(message),
        ErrorKind::Internal => Error::internal(message),
    };
    fail(error)
}
