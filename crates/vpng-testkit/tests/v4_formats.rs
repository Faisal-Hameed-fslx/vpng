//! The v0.4 file format: named layers and masks.
//!
//! These tests pin the rules that make a v0.4 file safe:
//!
//! * a file that carries a layer table says so in its minor version;
//! * a file with layers may **not** also have a preview pyramid;
//! * every layer's pixels are recovered **exactly**, on their own;
//! * a file with one layer composites to exactly that layer's pixels;
//! * a mask is a mask layer, its alpha is the coverage, and it modulates
//!   the layer it masks;
//! * the layer table's byte accounting matches the directory exactly;
//! * the default path still writes the v0.1 bytes.

use vpng_container::{Directory, Header, LayerTable, HEADER_SIZE, MINOR_V0_1, MINOR_V0_4};
use vpng_core::{CanonicalImage, ErrorKind, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_encoder::{encode, encode_layers, EncodeOptions, LayerInput};

/// A picture with enough structure that every codec has something to chew on.
fn picture(width: u32, height: u32) -> CanonicalImage {
    let bpp = 4usize;
    let mut data = vec![0u8; (width * height) as usize * bpp];
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) as usize) * bpp;
            if x < width / 2 {
                data[base] = 0x20;
                data[base + 1] = 0x30;
                data[base + 2] = 0x40;
            } else {
                for c in 0..3 {
                    state ^= state >> 12;
                    state ^= state << 25;
                    state ^= state >> 27;
                    data[base + c] = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u8;
                }
            }
            data[base + 3] = 0xFF;
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// A picture whose alpha channel ramps from 0 to 255 across its width.
fn coverage(width: u32, height: u32) -> CanonicalImage {
    let bpp = 4usize;
    let mut data = vec![0u8; (width * height) as usize * bpp];
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) as usize) * bpp;
            data[base] = 0x10;
            data[base + 1] = 0x20;
            data[base + 2] = 0x30;
            data[base + 3] = if width == 1 {
                255
            } else {
                (x * 255 / (width - 1)) as u8
            };
        }
    }
    CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), data).unwrap()
}

/// The whole canvas.
fn canvas(width: u32, height: u32) -> Rect {
    Rect::new(0, 0, width, height).unwrap()
}

/// Encode a base layer with one extra layer on top of it.
fn two_layers(
    base: &CanonicalImage,
    overlay: &CanonicalImage,
    overlay_rect: Rect,
) -> vpng_encoder::EncodeReport {
    let layers = [
        LayerInput::pixels("base", canvas(base.width(), base.height()), base),
        LayerInput::pixels("overlay", overlay_rect, overlay),
    ];
    encode_layers(&layers, &EncodeOptions::default().with_tile_size(64, 64)).unwrap()
}

#[test]
fn the_default_path_is_still_a_v0_1_file() {
    // The layers milestone must not move a single published byte.
    let image = picture(300, 180);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_1);
    assert_eq!(header.feature_flags, 0);
    assert_eq!(header.layer_table_length, 0);
}

#[test]
fn a_layered_file_is_a_v0_4_file() {
    let base = picture(300, 180);
    let overlay = picture(100, 60);
    let report = two_layers(&base, &overlay, Rect::new(20, 20, 100, 60).unwrap());

    let header = Header::parse(&report.bytes).unwrap();
    assert_eq!(header.minor, MINOR_V0_4);
    assert!(header.has_layers());
    assert!(header.layer_table_length > 0);
    assert_eq!(report.layer_count, 2);
}

#[test]
fn a_layered_file_may_not_also_have_a_pyramid() {
    let base = picture(128, 96);
    let overlay = picture(64, 32);
    let layers = [
        LayerInput::pixels("base", canvas(128, 96), &base),
        LayerInput::pixels("overlay", Rect::new(0, 0, 64, 32).unwrap(), &overlay),
    ];
    let options = EncodeOptions::default()
        .with_tile_size(64, 64)
        .with_pyramid(2);
    let err = encode_layers(&layers, &options).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn every_layer_decodes_exactly() {
    let base = picture(300, 180);
    let overlay = picture(100, 60);
    let report = two_layers(&base, &overlay, Rect::new(20, 20, 100, 60).unwrap());

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.layer_count(), 2);

    let decoded_base = document.decode_layer(0, &limits).unwrap();
    assert_eq!(decoded_base.width(), 300);
    assert_eq!(decoded_base.height(), 180);
    assert_eq!(decoded_base.data(), base.data());

    let decoded_overlay = document.decode_layer(1, &limits).unwrap();
    assert_eq!(decoded_overlay.width(), 100);
    assert_eq!(decoded_overlay.height(), 60);
    assert_eq!(decoded_overlay.data(), overlay.data());
}

#[test]
fn a_one_layer_file_composites_to_exactly_its_pixels() {
    // The blend of a single layer is the identity, not an approximation.
    let base = picture(300, 180);
    let layers = [LayerInput::pixels("base", canvas(300, 180), &base)];
    let report = encode_layers(&layers, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.layer_count(), 1);
    let composite = document.composite(&limits).unwrap();
    assert_eq!(composite.data(), base.data());
}

#[test]
fn the_base_layer_must_cover_the_whole_canvas() {
    let base = picture(300, 180);
    let layers = [LayerInput::pixels(
        "base",
        Rect::new(0, 0, 100, 100).unwrap(),
        &base,
    )];
    let err = encode_layers(&layers, &EncodeOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_layer_may_not_leave_the_canvas() {
    let base = picture(300, 180);
    let overlay = picture(100, 60);
    let layers = [
        LayerInput::pixels("base", canvas(300, 180), &base),
        LayerInput::pixels("overlay", Rect::new(250, 150, 100, 60).unwrap(), &overlay),
    ];
    let err = encode_layers(&layers, &EncodeOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_layer_must_hold_the_pixels_its_rectangle_says() {
    let base = picture(300, 180);
    let overlay = picture(100, 60);
    let layers = [
        LayerInput::pixels("base", canvas(300, 180), &base),
        LayerInput::pixels("overlay", Rect::new(0, 0, 120, 60).unwrap(), &overlay),
    ];
    let err = encode_layers(&layers, &EncodeOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn the_layer_table_accounts_for_every_byte() {
    let base = picture(300, 180);
    let overlay = picture(100, 60);
    let report = two_layers(&base, &overlay, Rect::new(20, 20, 100, 60).unwrap());

    let header = Header::parse(&report.bytes).unwrap();
    let start = header.layer_table_offset as usize;
    let end = start + header.layer_table_length as usize;
    let table = LayerTable::decode(&report.bytes[start..end]).unwrap();
    assert_eq!(table.len(), 2);
    assert_eq!(table.layers[0].name, "base");
    assert_eq!(table.layers[1].name, "overlay");

    // The table's accounting matches the encoder's report exactly.
    assert_eq!(
        table.layers[0].uncompressed_bytes,
        report.layer_uncompressed_bytes[0]
    );
    assert_eq!(
        table.layers[0].compressed_bytes,
        report.layer_compressed_bytes[0]
    );
    assert_eq!(
        table.layers[1].uncompressed_bytes,
        report.layer_uncompressed_bytes[1]
    );
    assert_eq!(
        table.layers[1].compressed_bytes,
        report.layer_compressed_bytes[1]
    );

    // And the bytes add up: the two layers' payloads plus the framing make
    // the file.
    let payloads: u64 = report.layer_compressed_bytes.iter().sum();
    assert_eq!(
        report.file_size,
        (HEADER_SIZE as u64)
            + (report.tile_count as u64 * 40 + 4)
            + header.layer_table_length as u64
            + payloads
    );
}

#[test]
fn a_mask_is_a_mask_layer_and_needs_an_alpha_channel() {
    let base = picture(300, 180);
    let mask = coverage(300, 180);

    // A mask that is not a mask layer is refused.
    let layers = [
        LayerInput::pixels("base", canvas(300, 180), &base),
        LayerInput::pixels("not a mask", canvas(300, 180), &mask).with_mask(1),
    ];
    let err = encode_layers(&layers, &EncodeOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);

    // A mask in a format with no alpha channel is refused.
    let grey = CanonicalImage::from_vec(64, 64, PixelFormat::gray8(), vec![0u8; 4096]).unwrap();
    let layers = [
        LayerInput::pixels("base", canvas(64, 64), &grey),
        LayerInput::mask("m", canvas(64, 64), &grey),
    ];
    let err = encode_layers(&layers, &EncodeOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
}

#[test]
fn a_mask_must_cover_the_layer_it_masks() {
    let base = picture(300, 180);
    let overlay = picture(100, 60);
    let mask = coverage(50, 30);
    let layers = [
        LayerInput::pixels("base", canvas(300, 180), &base),
        LayerInput::pixels("overlay", Rect::new(20, 20, 100, 60).unwrap(), &overlay).with_mask(2),
        LayerInput::mask("m", Rect::new(20, 20, 50, 30).unwrap(), &mask),
    ];
    let err = encode_layers(&layers, &EncodeOptions::default()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_mask_is_never_drawn_on_its_own() {
    let base = picture(300, 180);
    let mask = coverage(300, 180);
    let layers = [
        LayerInput::pixels("base", canvas(300, 180), &base),
        LayerInput::mask("m", canvas(300, 180), &mask),
    ];
    let report = encode_layers(&layers, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.layer_count(), 2);
    assert!(document.layer(1).unwrap().is_mask());

    // The composite is the base layer alone: a mask modulates, it does not
    // draw.
    let composite = document.composite(&limits).unwrap();
    assert_eq!(composite.data(), base.data());
}

#[test]
fn a_mask_modulates_the_layer_it_masks() {
    // The base is opaque white; the overlay is opaque black; the mask's
    // alpha ramps from 0 to 255. The composite must be a ramp from white to
    // black — never something else.
    let width = 64u32;
    let height = 32u32;
    let white = CanonicalImage::from_vec(
        width,
        height,
        PixelFormat::rgba8(),
        vec![0xFF; (width * height * 4) as usize],
    )
    .unwrap();
    // Opaque black: the overlay must actually cover the base where the
    // mask says it does.
    let mut black_data = vec![0u8; (width * height * 4) as usize];
    for pixel in black_data.chunks_exact_mut(4) {
        pixel[3] = 0xFF;
    }
    let black = CanonicalImage::from_vec(width, height, PixelFormat::rgba8(), black_data).unwrap();
    let mask = coverage(width, height);

    let layers = [
        LayerInput::pixels("base", canvas(width, height), &white),
        LayerInput::pixels("ink", canvas(width, height), &black).with_mask(2),
        LayerInput::mask("m", canvas(width, height), &mask),
    ];
    let report = encode_layers(&layers, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();

    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    let composite = document.composite(&limits).unwrap();

    for x in 0..width as usize {
        let base = x * 4;
        let expected = 255 - (x * 255 / (width as usize - 1));
        assert_eq!(
            composite.data()[base],
            expected as u8,
            "the mask's coverage must modulate the layer at column {x}"
        );
        // The overlay's own alpha is opaque, so the composite's alpha is too.
        assert_eq!(composite.data()[base + 3], 0xFF);
    }
}

#[test]
fn the_layer_index_travels_in_the_directory_flags() {
    let base = picture(300, 180);
    let overlay = picture(100, 60);
    let report = two_layers(&base, &overlay, Rect::new(20, 20, 100, 60).unwrap());

    let header = Header::parse(&report.bytes).unwrap();
    assert!(header.has_layers());
    let directory = Directory::decode(
        &report.bytes[HEADER_SIZE..HEADER_SIZE + report.tile_count as usize * 40 + 4],
        report.tile_count,
    )
    .unwrap();

    let layers: Vec<u16> = directory
        .entries
        .iter()
        .map(|entry| entry.layer())
        .collect();
    assert!(layers.iter().all(|&layer| layer == 0 || layer == 1));
    assert_eq!(layers[0], 0);
    assert_eq!(*layers.last().unwrap(), 1);
    // The entries are in canonical `(layer, level, y, x)` order.
    let mut sorted = layers.clone();
    sorted.sort_unstable();
    assert_eq!(layers, sorted);
}

#[test]
fn a_v0_3_file_may_not_carry_a_layer_table() {
    let base = picture(128, 96);
    let overlay = picture(64, 32);
    let report = two_layers(&base, &overlay, Rect::new(0, 0, 64, 32).unwrap());

    // Claim an older version and repair the header checksum: the reader must
    // then refuse the file.
    let mut bytes = report.bytes.clone();
    bytes[10..12].copy_from_slice(&MINOR_V0_1.to_le_bytes());
    let sum = vpng_container::checksum::crc32c_parts(&[&bytes[0..52], &bytes[56..64]]);
    bytes[52..56].copy_from_slice(&sum.to_le_bytes());

    let limits = Limits::default();
    let err = Document::parse(&bytes, &limits).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::CorruptFile);
}

#[test]
fn a_file_without_a_layer_table_has_one_layer() {
    // Every file written before v0.4 has exactly one layer, so a caller never
    // has to ask "is this a layered file?" before asking "what is its base
    // layer?".
    let image = picture(300, 180);
    let report = encode(&image, &EncodeOptions::default().with_tile_size(64, 64)).unwrap();
    let limits = Limits::default();
    let document = Document::parse(&report.bytes, &limits).unwrap();
    assert_eq!(document.layer_count(), 1);
    assert_eq!(document.layer(0).unwrap().name(), "");
    assert_eq!(
        document.decode_layer(0, &limits).unwrap().data(),
        image.data()
    );
    assert_eq!(document.composite(&limits).unwrap().data(), image.data());
}
