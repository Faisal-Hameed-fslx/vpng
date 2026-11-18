//! A minimal JSON writer.
//!
//! The validation tools write their reports as JSON so a script can read
//! them without a JSON library in the dependency tree. The writer is
//! deliberately small: it holds a document in memory and serializes it with
//! stable key order.

use std::fmt::Write as _;

/// A JSON value.
#[derive(Debug, Clone)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// An unsigned integer.
    UInt(u64),
    /// A floating point number.
    Num(f64),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Json>),
    /// An object. The key order is the order of the vector.
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Build a string value.
    pub fn string(text: impl Into<String>) -> Json {
        Json::Str(text.into())
    }

    /// Build an array of strings.
    pub fn strings<I: IntoIterator<Item = S>, S: Into<String>>(items: I) -> Json {
        Json::Arr(
            items
                .into_iter()
                .map(|item| Json::Str(item.into()))
                .collect(),
        )
    }

    /// Serialize the value.
    pub fn to_string_pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    /// Write the value into a string.
    pub fn write(&self, out: &mut String, depth: usize) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Json::Int(value) => {
                let _ = write!(out, "{value}");
            }
            Json::UInt(value) => {
                let _ = write!(out, "{value}");
            }
            Json::Num(value) => {
                if value.is_finite() {
                    let _ = write!(out, "{value:.6}");
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(text) => write_string(out, text),
            Json::Arr(items) => {
                if items.is_empty() {
                    out.push_str("[]");
                    return;
                }
                out.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    indent(out, depth + 1);
                    item.write(out, depth + 1);
                    if index + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(out, depth);
                out.push(']');
            }
            Json::Obj(fields) => {
                if fields.is_empty() {
                    out.push_str("{}");
                    return;
                }
                out.push_str("{\n");
                for (index, (key, value)) in fields.iter().enumerate() {
                    indent(out, depth + 1);
                    write_string(out, key);
                    out.push_str(": ");
                    value.write(out, depth + 1);
                    if index + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(out, depth);
                out.push('}');
            }
        }
    }
}

/// Write an indentation.
fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Write a JSON string.
fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
