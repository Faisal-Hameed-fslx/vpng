//! The protocol between the window and the session.
//!
//! The window is a browser; the session is Rust. The two speak JSON over
//! HTTP. This module is the **only** place that knows the shape of those
//! messages, so the session itself stays free of protocol concerns and the
//! messages have one definition rather than two.
//!
//! # The rules
//!
//! * A rectangle is always four whole numbers: `x`, `y`, `width` and
//!   `height`. A rectangle that is not a rectangle — a zero width, a
//!   negative value, a missing field — is refused before it reaches the
//!   engine.
//! * A colour is always four bytes: `r`, `g`, `b` and `a`.
//! * An error is always the same shape: a `kind` from the shared
//!   [`vpng_core::ErrorKind`] and a `message`. The window shows the message
//!   and never has to parse it.
//! * A **display** response is not JSON: it is the region's pixels, as
//!   RGBA8, with the geometry in the response's headers. A picture is not a
//!   JSON document, and encoding one as base64 would cost a third more
//!   memory and a third more time.

use vpng_core::{Error, ErrorKind, Rect, Result};
use vpng_json::Json;

use crate::session::{Colour, EditReport, Pixel, SessionInfo, Tool};

/// Build a JSON object from a list of entries.
fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

/// A rectangle in the picture's own pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectSpec {
    /// The left edge.
    pub x: u32,
    /// The top edge.
    pub y: u32,
    /// The width.
    pub width: u32,
    /// The height.
    pub height: u32,
}

impl RectSpec {
    /// Read a rectangle from a JSON object.
    pub fn from_json(value: &Json) -> Result<Self> {
        if value.as_object().is_none() {
            return Err(Error::invalid(format!(
                "a rectangle must be an object, not {}",
                value.kind()
            )));
        }
        let x = field(value, "x")?;
        let y = field(value, "y")?;
        let width = field(value, "width")?;
        let height = field(value, "height")?;
        let spec = RectSpec {
            x,
            y,
            width,
            height,
        };
        if spec.width == 0 || spec.height == 0 {
            return Err(Error::invalid(
                "a rectangle must have a width and a height of at least one pixel",
            ));
        }
        Ok(spec)
    }

    /// The rectangle.
    pub fn to_rect(self) -> Result<Rect> {
        Rect::new(self.x, self.y, self.width, self.height)
    }
}

/// Read one whole number from a JSON object.
fn field(value: &Json, name: &str) -> Result<u32> {
    let raw = value
        .get(name)
        .ok_or_else(|| Error::invalid(format!("the request has no `{name}` field")))?;
    let number = raw
        .as_u64()
        .ok_or_else(|| Error::invalid(format!("the `{name}` field must be a whole number")))?;
    u32::try_from(number).map_err(|_| Error::invalid(format!("the `{name}` field is too large")))
}

/// Read one colour from a JSON object.
pub fn colour_from_json(value: &Json) -> Result<Colour> {
    if value.as_object().is_none() {
        return Err(Error::invalid(format!(
            "a colour must be an object, not {}",
            value.kind()
        )));
    }
    Ok(Colour {
        red: byte_field(value, "r")?,
        green: byte_field(value, "g")?,
        blue: byte_field(value, "b")?,
        alpha: byte_field(value, "a")?,
    })
}

/// Read one byte from a JSON object.
fn byte_field(value: &Json, name: &str) -> Result<u8> {
    let number = field(value, name)?;
    u8::try_from(number)
        .map_err(|_| Error::invalid(format!("the `{name}` field must be between 0 and 255")))
}

/// The colour, as JSON.
pub fn colour_to_json(colour: Colour) -> Json {
    object(vec![
        ("r", Json::from(colour.red)),
        ("g", Json::from(colour.green)),
        ("b", Json::from(colour.blue)),
        ("a", Json::from(colour.alpha)),
    ])
}

/// The session's state, as JSON.
pub fn state_json(info: &SessionInfo) -> Json {
    object(vec![
        ("path", Json::from(info.path.to_string_lossy().into_owned())),
        ("width", Json::from(info.width)),
        ("height", Json::from(info.height)),
        (
            "format",
            Json::from(crate::session::format_name(info.format)),
        ),
        ("channels", Json::from(info.format.channels())),
        ("bits", Json::from(info.format.bits.bits())),
        ("has_alpha", Json::from(info.format.color_model.has_alpha())),
        ("tile_count", Json::from(info.tile_count)),
        ("level_count", Json::from(info.level_count)),
        ("tool", Json::from(info.tool.name())),
        ("colour", colour_to_json(info.colour)),
        ("dirty", Json::from(info.dirty)),
        ("dirty_tiles", Json::from(info.dirty_tiles)),
        ("edits", Json::from(info.edits)),
        ("can_undo", Json::from(info.can_undo)),
        ("can_redo", Json::from(info.can_redo)),
        ("max_region_pixels", Json::from(info.max_region_pixels)),
        ("cache_budget", Json::from(info.cache_budget)),
        ("cache_current_bytes", Json::from(info.cache_current_bytes)),
        ("history_budget", Json::from(info.history_budget)),
        (
            "history_current_bytes",
            Json::from(info.history_current_bytes),
        ),
        ("visible_tiles", Json::from(info.plan.visible_tiles)),
        ("prefetch_tiles", Json::from(info.plan.prefetch_tiles)),
        ("prefetched_tiles", Json::from(info.plan.prefetched_tiles)),
    ])
}

/// What an edit did, as JSON.
pub fn edit_json(report: &EditReport) -> Json {
    object(vec![
        ("pixels", Json::from(report.pixels)),
        ("dirty_tiles", Json::from(report.dirty_tiles)),
    ])
}

/// One pixel, as JSON.
pub fn pixel_json(pixel: &Pixel) -> Json {
    let raw = Json::Array(pixel.raw.iter().map(|value| Json::from(*value)).collect());
    object(vec![
        ("r", Json::from(pixel.red)),
        ("g", Json::from(pixel.green)),
        ("b", Json::from(pixel.blue)),
        ("a", Json::from(pixel.alpha)),
        ("raw", raw),
    ])
}

/// An error, as JSON.
pub fn error_json(error: &Error) -> Json {
    object(vec![(
        "error",
        object(vec![
            ("kind", Json::from(error.kind().label())),
            ("message", Json::from(error.to_string())),
            ("exit_code", Json::from(error.exit_code() as u64)),
        ]),
    )])
}

/// The tool a request names.
pub fn tool_from_json(value: &Json) -> Result<Tool> {
    let name = value
        .get("tool")
        .and_then(Json::as_str)
        .ok_or_else(|| Error::invalid("the request has no `tool` field"))?;
    Tool::from_name(name)
}

/// The path a save request names, if it names one.
pub fn path_from_json(value: &Json) -> Result<Option<String>> {
    match value.get("path") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(Error::invalid(format!(
            "the `path` field must be a string, not {}",
            other.kind()
        ))),
    }
}

/// The error kind's label, for a message that says what went wrong.
pub fn kind_label(kind: ErrorKind) -> &'static str {
    kind.label()
}
