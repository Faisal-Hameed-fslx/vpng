//! A small, strict JSON reader.
//!
//! The CLI needs to read exactly one kind of JSON: the **layer manifest**,
//! which says how to build a layered file. Rather than take a dependency on a
//! JSON crate — and rather than hand-roll a fragile ad-hoc parser for one
//! file format — this is a complete, small JSON reader with the strictness a
//! configuration file needs:
//!
//! * no trailing commas;
//! * no comments;
//! * no unquoted keys, no single-quoted strings;
//! * no `NaN`, no `Infinity`;
//! * every number is finite.
//!
//! It parses into a [`Json`] tree, and the tree answers the few questions
//! the manifest asks of it. The parser never executes anything and never
//! allocates beyond the text it was given.

use vpng_core::{Error, Result};

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, kept as the text it was written as.
    ///
    /// The text is kept so that an integer stays an integer: the manifest
    /// holds pixel coordinates, and a coordinate that has been through a
    /// floating-point number is no longer exactly the coordinate the author
    /// wrote.
    Number(String),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Json>),
    /// An object. The entries keep the order they were written in.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value of a key, if this is an object that has one.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The string, if this is a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    /// The number, if this is a number that is a whole number.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Number(text) => text.parse::<u64>().ok(),
            _ => None,
        }
    }

    /// The elements, if this is an array.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The entries, if this is an object.
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// Whether this is `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// The name of this value's kind, for an error message.
    pub fn kind(&self) -> &'static str {
        match self {
            Json::Null => "null",
            Json::Bool(_) => "a boolean",
            Json::Number(_) => "a number",
            Json::String(_) => "a string",
            Json::Array(_) => "an array",
            Json::Object(_) => "an object",
        }
    }
}

/// Parse a JSON document.
pub fn parse(text: &str) -> Result<Json> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        position: 0,
    };
    parser.skip_whitespace();
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err(parser.error("there is text after the JSON value"));
    }
    Ok(value)
}

/// The parser's state.
struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    /// The error, with the position where it happened.
    fn error(&self, message: &str) -> Error {
        Error::invalid(format!("invalid JSON at byte {}: {message}", self.position))
    }

    /// Skip the whitespace between values.
    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.position) {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' => self.position += 1,
                _ => break,
            }
        }
    }

    /// The byte at the cursor, or `None` at the end of the text.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    /// Expect a byte and move past it.
    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.peek() == Some(byte) {
            self.position += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected `{}`", byte as char)))
        }
    }

    /// One value.
    fn value(&mut self) -> Result<Json> {
        match self.peek() {
            None => Err(self.error("the text is empty")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.number(),
            Some(_) => Err(self.error("there is a character that cannot start a value")),
        }
    }

    /// One of the three words JSON has.
    fn literal(&mut self, word: &str, value: Json) -> Result<Json> {
        if self.bytes[self.position..].starts_with(word.as_bytes()) {
            self.position += word.len();
            Ok(value)
        } else {
            Err(self.error(&format!("expected `{word}`")))
        }
    }

    /// A number, kept as its text.
    fn number(&mut self) -> Result<Json> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        // The integer part: either a lone `0` or a digit sequence that does
        // not start with a `0`.
        match self.peek() {
            Some(b'0') => self.position += 1,
            Some(byte) if byte.is_ascii_digit() => {
                while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                    self.position += 1;
                }
            }
            _ => return Err(self.error("expected a digit")),
        }
        // The fraction.
        if self.peek() == Some(b'.') {
            self.position += 1;
            if !matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                return Err(self.error("expected a digit after the decimal point"));
            }
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.position += 1;
            }
        }
        // The exponent.
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.position += 1;
            }
            if !matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                return Err(self.error("expected a digit in the exponent"));
            }
            while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
                self.position += 1;
            }
        }

        let text = std::str::from_utf8(&self.bytes[start..self.position])
            .map_err(|_| self.error("the number is not valid UTF-8"))?;
        Ok(Json::Number(text.to_string()))
    }

    /// A string.
    fn string(&mut self) -> Result<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = match self.peek() {
                Some(byte) => byte,
                None => return Err(self.error("the string is not closed")),
            };
            self.position += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("the escape is not finished"))?;
                    self.position += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("there is an unknown escape")),
                    }
                }
                byte if byte < 0x20 => {
                    return Err(self.error("a control character must be escaped"))
                }
                byte => {
                    // The text is UTF-8, so a byte that is not a control
                    // character belongs to a character: copy the whole
                    // character, not the byte.
                    let start = self.position - 1;
                    let width = utf8_width(byte);
                    let end = start + width;
                    if end > self.bytes.len() {
                        return Err(self.error("the text is not valid UTF-8"));
                    }
                    let chunk = std::str::from_utf8(&self.bytes[start..end])
                        .map_err(|_| self.error("the text is not valid UTF-8"))?;
                    out.push_str(chunk);
                    self.position = end;
                }
            }
        }
    }

    /// A `\u` escape, including a surrogate pair.
    fn unicode_escape(&mut self) -> Result<char> {
        let first = self.hex4()?;
        if (0xD800..0xDC00).contains(&first) {
            // A high surrogate: the low one must follow.
            self.expect(b'\\')?;
            self.expect(b'u')?;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.error("the escape is not a valid surrogate pair"));
            }
            let code = 0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00);
            return char::from_u32(code).ok_or_else(|| self.error("the escape is not a character"));
        }
        if (0xDC00..0xE000).contains(&first) {
            return Err(self.error("the escape is a low surrogate with no high one"));
        }
        char::from_u32(first).ok_or_else(|| self.error("the escape is not a character"))
    }

    /// Four hexadecimal digits.
    fn hex4(&mut self) -> Result<u32> {
        if self.position + 4 > self.bytes.len() {
            return Err(self.error("the escape is not finished"));
        }
        let text = std::str::from_utf8(&self.bytes[self.position..self.position + 4])
            .map_err(|_| self.error("the escape is not valid UTF-8"))?;
        let value = u32::from_str_radix(text, 16)
            .map_err(|_| self.error("the escape is not hexadecimal"))?;
        self.position += 4;
        Ok(value)
    }

    /// An array.
    fn array(&mut self) -> Result<Json> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    /// An object.
    fn object(&mut self) -> Result<Json> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }
}

/// How many bytes the character that starts with this byte occupies.
fn utf8_width(byte: u8) -> usize {
    match byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_object() {
        let value = parse(r#"{"name": "base", "x": 10, "y": 0}"#).unwrap();
        assert_eq!(value.get("name").and_then(Json::as_str), Some("base"));
        assert_eq!(value.get("x").and_then(Json::as_u64), Some(10));
        assert_eq!(value.get("y").and_then(Json::as_u64), Some(0));
        assert!(value.get("missing").is_none());
    }

    #[test]
    fn reads_an_array() {
        let value = parse(r#"[1, 2, 3]"#).unwrap();
        let items = value.as_array().unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[2].as_u64(), Some(3));
    }

    #[test]
    fn keeps_a_number_as_its_text() {
        // A number that does not fit in a u64 is kept as the text it was
        // written as, and it is not an integer the caller can use.
        let value = parse("99999999999999999999999").unwrap();
        assert_eq!(value.as_u64(), None);
        assert_eq!(value, Json::Number("99999999999999999999999".to_string()));

        // A number that does fit is an integer.
        let value = parse("4294967295").unwrap();
        assert_eq!(value.as_u64(), Some(4294967295));
    }

    #[test]
    fn reads_the_escapes() {
        let value = parse(r#""a\"b\\c\ndAé""#).unwrap();
        assert_eq!(value.as_str(), Some("a\"b\\c\ndAé"));
    }

    #[test]
    fn reads_a_surrogate_pair() {
        let value = parse(r#""😀""#).unwrap();
        assert_eq!(value.as_str(), Some("😀"));
    }

    #[test]
    fn refuses_a_trailing_comma() {
        assert!(parse(r#"{"a": 1,}"#).is_err());
        assert!(parse("[1, 2,]").is_err());
    }

    #[test]
    fn refuses_a_comment() {
        assert!(parse(r#"{"a": 1} // hello"#).is_err());
    }

    #[test]
    fn refuses_trailing_text() {
        assert!(parse("{} {}").is_err());
    }

    #[test]
    fn refuses_an_unquoted_key() {
        assert!(parse("{a: 1}").is_err());
    }

    #[test]
    fn refuses_a_bare_word() {
        assert!(parse("hello").is_err());
    }

    #[test]
    fn refuses_an_unfinished_string() {
        assert!(parse(r#""hello"#).is_err());
    }

    #[test]
    fn refuses_a_control_character() {
        assert!(parse("\"a\nb\"").is_err());
    }

    #[test]
    fn reads_null_and_the_booleans() {
        assert!(parse("null").unwrap().is_null());
        assert_eq!(parse("true").unwrap(), Json::Bool(true));
        assert_eq!(parse("false").unwrap(), Json::Bool(false));
    }

    #[test]
    fn reads_an_empty_object_and_array() {
        assert_eq!(parse("{}").unwrap().as_object().unwrap().len(), 0);
        assert_eq!(parse("[]").unwrap().as_array().unwrap().len(), 0);
    }

    #[test]
    fn keeps_the_order_of_the_entries() {
        let value = parse(r#"{"b": 1, "a": 2}"#).unwrap();
        let entries = value.as_object().unwrap();
        assert_eq!(entries[0].0, "b");
        assert_eq!(entries[1].0, "a");
    }
}
