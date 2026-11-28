//! The editing session must change exactly what the user asked for.
//!
//! Every test here states the pixels the file must hold after the edit, not
//! merely that the file changed. The editor is an editor: a fill that
//! changes a pixel the user did not ask about is a defect, and a 16-bit file
//! that is quietly written as 8-bit is a worse one.

mod common;

use common::{expected, gradient, scratch, write_picture};
use vpng_core::{ErrorKind, Limits, PixelFormat, Rect};
use vpng_decoder::Document;
use vpng_ui::{Colour, Session, Tool};

/// Every format the editor can show.
fn formats() -> Vec<PixelFormat> {
    vec![
        PixelFormat::gray8(),
        PixelFormat::gray_alpha8(),
        PixelFormat::rgb8(),
        PixelFormat::rgba8(),
        PixelFormat::gray16(),
        PixelFormat::gray_alpha16(),
        PixelFormat::rgb16(),
        PixelFormat::rgba16(),
    ]
}

#[test]
fn a_fresh_session_reports_the_file() {
    let directory = scratch("fresh");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgb8()),
    );
    let session = Session::open(&path, &Limits::default()).unwrap();
    let info = session.info();
    assert_eq!(info.width, 64);
    assert_eq!(info.height, 48);
    assert_eq!(info.format, PixelFormat::rgb8());
    assert_eq!(info.tile_count, 1);
    assert!(!info.dirty);
    assert_eq!(info.edits, 0);
    assert!(!info.can_undo);
    assert!(!info.can_redo);
    assert_eq!(info.tool, Tool::Pan);
}

#[test]
fn a_region_read_returns_the_pixels_exactly() {
    for format in formats() {
        let directory = scratch("region");
        let path = write_picture(&directory, "picture.vpng", &gradient(64, 48, format));
        let mut session = Session::open(&path, &Limits::default()).unwrap();

        let rect = Rect::new(10, 12, 8, 6).unwrap();
        let pixels = session.region(rect).unwrap();
        assert_eq!(pixels.len(), 8 * 6 * 4, "{format:?}");

        for y in 0..6u32 {
            for x in 0..8u32 {
                let offset = ((y * 8 + x) * 4) as usize;
                let got = [
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ];
                let want = expected(10 + x, 12 + y, format);
                assert_eq!(got, want, "{format:?} at ({x}, {y})");
            }
        }
    }
}

#[test]
fn a_pick_returns_the_pixels_exactly() {
    for format in formats() {
        let directory = scratch("pick");
        let path = write_picture(&directory, "picture.vpng", &gradient(64, 48, format));
        let mut session = Session::open(&path, &Limits::default()).unwrap();

        let pixel = session.pick(7, 9).unwrap();
        let want = expected(7, 9, format);
        assert_eq!(
            [pixel.red, pixel.green, pixel.blue, pixel.alpha],
            want,
            "{format:?}"
        );

        // The raw samples are the file's own, so a 16-bit file reports 16-bit
        // values.
        if format.bits.bits() == 16 {
            assert!(pixel.raw.iter().all(|value| *value >= 255));
        }
    }
}

#[test]
fn a_fill_changes_exactly_the_rectangle() {
    for format in formats() {
        let directory = scratch("fill");
        let path = write_picture(&directory, "picture.vpng", &gradient(64, 48, format));
        let mut session = Session::open(&path, &Limits::default()).unwrap();

        session.set_colour(Colour {
            red: 200,
            green: 100,
            blue: 50,
            alpha: 128,
        });
        let rect = Rect::new(20, 20, 8, 6).unwrap();
        let report = session.fill(rect).unwrap();
        assert_eq!(report.pixels, 48);
        assert_eq!(report.dirty_tiles, 1);
        assert!(session.info().dirty);

        // The whole picture, read back, must agree with the original outside
        // the rectangle and with the colour inside it.
        let whole = session.region(Rect::new(0, 0, 64, 48).unwrap()).unwrap();
        let has_alpha = format.color_model.has_alpha();
        let grey = (200u32 + 100 + 50 + 1) / 3;
        for y in 0..48u32 {
            for x in 0..64u32 {
                let offset = ((y * 64 + x) * 4) as usize;
                let got = [
                    whole[offset],
                    whole[offset + 1],
                    whole[offset + 2],
                    whole[offset + 3],
                ];
                let inside = (20..28).contains(&x) && (20..26).contains(&y);
                if inside {
                    let want = match format.color_model {
                        vpng_core::ColorModel::Gray => [grey as u8, grey as u8, grey as u8, 255],
                        vpng_core::ColorModel::GrayAlpha => {
                            [grey as u8, grey as u8, grey as u8, 128]
                        }
                        vpng_core::ColorModel::Rgb => [200, 100, 50, 255],
                        vpng_core::ColorModel::Rgba => [200, 100, 50, 128],
                        vpng_core::ColorModel::Indexed => unreachable!(),
                    };
                    assert_eq!(got, want, "{format:?} inside at ({x}, {y})");
                } else {
                    let want = expected(x, y, format);
                    assert_eq!(got, want, "{format:?} outside at ({x}, {y})");
                }
                let _ = has_alpha;
            }
        }
    }
}

#[test]
fn a_fill_is_exactly_undoable() {
    let directory = scratch("undo");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgba8()),
    );
    let mut session = Session::open(&path, &Limits::default()).unwrap();
    let before = session.region(Rect::new(0, 0, 64, 48).unwrap()).unwrap();

    session.set_colour(Colour {
        red: 1,
        green: 2,
        blue: 3,
        alpha: 4,
    });
    session.fill(Rect::new(10, 10, 20, 20).unwrap()).unwrap();
    assert_ne!(
        session.region(Rect::new(0, 0, 64, 48).unwrap()).unwrap(),
        before
    );

    assert!(session.undo().unwrap());
    assert_eq!(
        session.region(Rect::new(0, 0, 64, 48).unwrap()).unwrap(),
        before
    );
    assert!(!session.info().dirty);

    assert!(session.redo().unwrap());
    assert_ne!(
        session.region(Rect::new(0, 0, 64, 48).unwrap()).unwrap(),
        before
    );
}

#[test]
fn an_alpha_edit_is_refused_when_the_file_has_no_alpha() {
    let directory = scratch("no-alpha");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgb8()),
    );
    let mut session = Session::open(&path, &Limits::default()).unwrap();
    let error = session
        .set_alpha(Rect::new(0, 0, 4, 4).unwrap(), 128)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFeature);
    assert!(error.to_string().contains("alpha"), "{error}");
}

#[test]
fn an_alpha_edit_sets_the_alpha_channel() {
    let directory = scratch("alpha");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgba8()),
    );
    let mut session = Session::open(&path, &Limits::default()).unwrap();
    session
        .set_alpha(Rect::new(4, 4, 6, 6).unwrap(), 77)
        .unwrap();

    let pixels = session.region(Rect::new(0, 0, 64, 48).unwrap()).unwrap();
    for y in 0..48u32 {
        for x in 0..64u32 {
            let offset = ((y * 64 + x) * 4) as usize;
            let inside = (4..10).contains(&x) && (4..10).contains(&y);
            let want = if inside { 77 } else { 255 };
            assert_eq!(pixels[offset + 3], want, "at ({x}, {y})");
        }
    }
}

#[test]
fn a_sixteen_bit_file_stays_sixteen_bit() {
    let directory = scratch("sixteen");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgba16()),
    );
    let mut session = Session::open(&path, &Limits::default()).unwrap();
    session.set_colour(Colour {
        red: 255,
        green: 128,
        blue: 0,
        alpha: 255,
    });
    session.fill(Rect::new(0, 0, 4, 4).unwrap()).unwrap();

    // The file's own samples must be the 16-bit ones: 255 becomes 65535.
    let pixel = session.pick(1, 1).unwrap();
    assert_eq!(pixel.raw, vec![65535, 128 * 257, 0, 65535]);
}

#[test]
fn a_region_beyond_the_limit_is_refused() {
    let directory = scratch("limit");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgb8()),
    );
    let mut session = Session::open(&path, &Limits::default()).unwrap();
    session.set_max_region_pixels(10).unwrap();
    let error = session.region(Rect::new(0, 0, 8, 8).unwrap()).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    assert!(error.to_string().contains("limit"), "{error}");
}

#[test]
fn a_rectangle_outside_the_picture_is_refused() {
    let directory = scratch("outside");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgb8()),
    );
    let mut session = Session::open(&path, &Limits::default()).unwrap();
    let error = session
        .region(Rect::new(60, 40, 10, 10).unwrap())
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidArgument);
}

#[test]
fn a_save_writes_the_picture_and_keeps_the_untouched_tiles() {
    let directory = scratch("save");
    // A picture with more than one tile, so the untouched ones can be seen.
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(600, 400, PixelFormat::rgb8()),
    );
    let original = std::fs::read(&path).unwrap();
    let original_document = Document::parse(&original, &Limits::default()).unwrap();
    let original_tiles = original_document.tile_count();
    assert!(original_tiles > 1, "the fixture must have several tiles");

    let mut session = Session::open(&path, &Limits::default()).unwrap();
    session.set_colour(Colour {
        red: 9,
        green: 8,
        blue: 7,
        alpha: 255,
    });
    session.fill(Rect::new(0, 0, 16, 16).unwrap()).unwrap();
    let report = session.save(None).unwrap();
    assert_eq!(report.tiles_rewritten, 1);
    assert_eq!(report.tiles_copied, original_tiles - 1);
    assert!(report.baseline_advanced);

    let saved = std::fs::read(&path).unwrap();
    let saved_document = Document::parse(&saved, &Limits::default()).unwrap();
    assert_eq!(saved_document.tile_count(), original_tiles);

    // Every tile the edit did not touch keeps its payload byte for byte.
    for index in 1..original_tiles as usize {
        assert_eq!(
            saved_document.tile_payload(index).unwrap(),
            original_document.tile_payload(index).unwrap(),
            "tile {index} changed"
        );
    }

    // The picture the saved file holds is the edited picture, exactly.
    let decoded = saved_document.decode_all(&Limits::default()).unwrap();
    let pixels = vpng_ui::to_rgba8(&decoded).unwrap();
    for y in 0..16u32 {
        for x in 0..16u32 {
            let offset = ((y * 600 + x) * 4) as usize;
            assert_eq!(
                [pixels[offset], pixels[offset + 1], pixels[offset + 2]],
                [9, 8, 7],
                "at ({x}, {y})"
            );
        }
    }
    for y in 0..400u32 {
        for x in 0..600u32 {
            if x < 16 && y < 16 {
                continue;
            }
            let offset = ((y * 600 + x) * 4) as usize;
            assert_eq!(
                [pixels[offset], pixels[offset + 1], pixels[offset + 2]],
                expected(x, y, PixelFormat::rgb8())[..3],
                "at ({x}, {y})"
            );
        }
    }
}

#[test]
fn a_saved_file_can_be_opened_again() {
    let directory = scratch("reopen");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgb8()),
    );
    let mut session = Session::open(&path, &Limits::default()).unwrap();
    session.set_colour(Colour {
        red: 11,
        green: 22,
        blue: 33,
        alpha: 255,
    });
    session.fill(Rect::new(0, 0, 4, 4).unwrap()).unwrap();
    session.save(None).unwrap();

    let reopened = Session::open(&path, &Limits::default()).unwrap();
    assert!(!reopened.info().dirty);
}
