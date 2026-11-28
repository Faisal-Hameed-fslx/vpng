//! The protocol's shape.
//!
//! The window and the engine agree on a handful of messages. Each of them
//! has one definition, and these tests pin that definition: a field that is
//! renamed, a number that turns into a float, an error that loses its kind
//! — each would break the window silently, so each is pinned here.

mod common;

use vpng_core::{Error, Limits, PixelFormat, Rect};
use vpng_json::Json;
use vpng_ui::protocol::{colour_from_json, colour_to_json, error_json, state_json, RectSpec};
use vpng_ui::{Colour, Session};

use common::{gradient, scratch, write_picture};

#[test]
fn a_rectangle_is_read_from_its_fields() {
    let value = vpng_json::parse(r#"{"x": 3, "y": 4, "width": 5, "height": 6}"#).unwrap();
    let spec = RectSpec::from_json(&value).unwrap();
    assert_eq!(spec.x, 3);
    assert_eq!(spec.y, 4);
    assert_eq!(spec.width, 5);
    assert_eq!(spec.height, 6);
    assert_eq!(spec.to_rect().unwrap(), Rect::new(3, 4, 5, 6).unwrap());
}

#[test]
fn a_malformed_rectangle_is_refused() {
    for text in [
        r#"{"x": 3, "y": 4, "width": 5}"#,
        r#"{"x": 3, "y": 4, "width": 0, "height": 6}"#,
        r#"{"x": 3, "y": 4, "width": -5, "height": 6}"#,
        r#"{"x": 1.5, "y": 4, "width": 5, "height": 6}"#,
        r#"[1, 2, 3]"#,
    ] {
        let value = vpng_json::parse(text).unwrap();
        assert!(
            RectSpec::from_json(&value).is_err(),
            "`{text}` was accepted"
        );
    }
}

#[test]
fn a_colour_is_read_and_written() {
    let value = vpng_json::parse(r#"{"r": 1, "g": 2, "b": 3, "a": 4}"#).unwrap();
    let colour = colour_from_json(&value).unwrap();
    assert_eq!(
        colour,
        Colour {
            red: 1,
            green: 2,
            blue: 3,
            alpha: 4
        }
    );

    let written = colour_to_json(colour);
    assert_eq!(written.to_string(), r#"{"r":1,"g":2,"b":3,"a":4}"#);
    assert_eq!(colour_from_json(&written).unwrap(), colour);
}

#[test]
fn a_colour_out_of_range_is_refused() {
    let value = vpng_json::parse(r#"{"r": 256, "g": 2, "b": 3, "a": 4}"#).unwrap();
    assert!(colour_from_json(&value).is_err());
    let value = vpng_json::parse(r#"{"r": 1, "g": 2, "b": 3}"#).unwrap();
    assert!(colour_from_json(&value).is_err());
}

#[test]
fn the_state_carries_what_the_window_needs() {
    let directory = scratch("state");
    let path = write_picture(
        &directory,
        "picture.vpng",
        &gradient(64, 48, PixelFormat::rgb8()),
    );
    let session = Session::open(&path, &Limits::default()).unwrap();
    let state = state_json(&session.info());

    assert_eq!(state.get("width").and_then(Json::as_u32), Some(64));
    assert_eq!(state.get("height").and_then(Json::as_u32), Some(48));
    assert_eq!(state.get("format").and_then(Json::as_str), Some("rgb8"));
    assert_eq!(state.get("tool").and_then(Json::as_str), Some("pan"));
    assert_eq!(state.get("dirty"), Some(&Json::Bool(false)));
    assert_eq!(state.get("tile_count").and_then(Json::as_u32), Some(1));
    assert_eq!(
        state.get("max_region_pixels").and_then(Json::as_u64),
        Some(vpng_ui::DEFAULT_MAX_REGION_PIXELS)
    );
    assert!(state.get("path").and_then(Json::as_str).is_some());
    assert!(state.get("colour").is_some());
}

#[test]
fn an_error_carries_its_kind_and_its_message() {
    let error = Error::invalid("the rectangle is outside the picture");
    let value = error_json(&error);
    let inner = value.get("error").expect("the error object");
    assert_eq!(
        inner.get("kind").and_then(Json::as_str),
        Some("invalid-argument")
    );
    assert!(inner
        .get("message")
        .and_then(Json::as_str)
        .unwrap()
        .contains("outside the picture"));
    assert_eq!(inner.get("exit_code").and_then(Json::as_u64), Some(2));
}
