//! A minimal HTTP/1.1 server, written for one local window.
//!
//! The editor needs a server that speaks just enough HTTP to carry a
//! browser's requests and this project's own messages. It is **not** a web
//! server: there is no keep-alive, no chunked transfer, no compression and
//! no virtual hosting. Every response closes its connection.
//!
//! Writing it here rather than taking a web framework has two reasons that
//! matter to this repository: the workspace keeps no asynchronous runtime and
//! no web framework, and the server's whole behaviour is then visible in one
//! file — including the limits, which are the interesting part.
//!
//! # The limits
//!
//! * A request head may not exceed **64 KiB**.
//! * A request body may not exceed **4 MiB**.
//! * A chunked body is refused: the browser's `fetch` sends a
//!   `Content-Length`, and a body that needs more machinery than that is
//!   not one this protocol sends.

use std::io::{BufRead, Write};

use vpng_core::{Error, Result};

/// The largest request head the server accepts.
pub const MAX_HEAD_BYTES: usize = 64 * 1024;

/// The largest request body the server accepts.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// A request the server has read.
#[derive(Debug, Clone)]
pub struct Request {
    /// The method, in upper case as it was written.
    pub method: String,
    /// The target, including the query string.
    pub target: String,
    /// The HTTP version.
    pub version: String,
    /// The headers, with their names in lower case.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl Request {
    /// The path, without the query string.
    pub fn path(&self) -> &str {
        match self.target.split_once('?') {
            Some((path, _)) => path,
            None => &self.target,
        }
    }

    /// The query string, without the `?`.
    pub fn query(&self) -> &str {
        match self.target.split_once('?') {
            Some((_, query)) => query,
            None => "",
        }
    }

    /// A header's value, by name. The name is matched in lower case.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The body, as text.
    pub fn body_text(&self) -> Result<String> {
        String::from_utf8(self.body.clone())
            .map_err(|_| Error::invalid("the request body is not valid UTF-8"))
    }
}

/// A response the server writes.
#[derive(Debug, Clone)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The headers.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl Response {
    /// A response with a body and a content type.
    pub fn new(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Response {
            status,
            headers: vec![("Content-Type".to_string(), content_type.to_string())],
            body,
        }
    }

    /// A response whose body is JSON.
    pub fn json(status: u16, value: &vpng_json::Json) -> Self {
        Response::new(status, "application/json", value.to_string().into_bytes())
    }

    /// A response with no body.
    pub fn empty(status: u16) -> Self {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Add a header.
    pub fn with_header(mut self, name: &str, value: impl std::fmt::Display) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

/// The reason phrase for a status code.
pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

/// Read one request from a reader.
///
/// The reader is a [`BufRead`], so the request head is read line by line and
/// the body is read exactly as long as the `Content-Length` says.
pub fn read_request(reader: &mut impl BufRead) -> Result<Request> {
    let mut head_bytes = 0usize;

    let line = read_line(reader, &mut head_bytes)?;
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| Error::invalid("the request has no method"))?
        .to_ascii_uppercase();
    let target = parts
        .next()
        .ok_or_else(|| Error::invalid("the request has no target"))?
        .to_string();
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();

    let mut headers = Vec::new();
    loop {
        let line = read_line(reader, &mut head_bytes)?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed
            .split_once(':')
            .ok_or_else(|| Error::invalid("a header line has no colon"))?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }

    if headers.iter().any(|(name, _)| name == "transfer-encoding") {
        return Err(Error::unsupported(
            "a chunked request body is not accepted; this protocol sends a Content-Length",
        ));
    }

    let length = request_content_length(&headers).unwrap_or_default();
    if length > MAX_BODY_BYTES {
        return Err(Error::resource_limit(format!(
            "the request body holds {length} bytes; the limit is {MAX_BODY_BYTES}"
        )));
    }

    let mut body = vec![0u8; length];
    if length > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|err| Error::invalid(format!("the request body is short: {err}")))?;
    }

    Ok(Request {
        method,
        target,
        version,
        headers,
        body,
    })
}

/// Read one line, counting the bytes read.
fn read_line(reader: &mut impl BufRead, head_bytes: &mut usize) -> Result<String> {
    let mut buffer = Vec::new();
    let read = reader
        .read_until(b'\n', &mut buffer)
        .map_err(|err| Error::invalid(format!("the request could not be read: {err}")))?;
    if read == 0 {
        return Err(Error::invalid(
            "the connection closed before the request was complete",
        ));
    }
    *head_bytes += read;
    if *head_bytes > MAX_HEAD_BYTES {
        return Err(Error::resource_limit(format!(
            "the request head is larger than {MAX_HEAD_BYTES} bytes"
        )));
    }
    String::from_utf8(buffer).map_err(|_| Error::invalid("the request head is not valid UTF-8"))
}

/// The `Content-Length` a request declares.
fn request_content_length(headers: &[(String, String)]) -> Option<usize> {
    headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
}

/// Write one response.
pub fn write_response(writer: &mut impl Write, response: &Response) -> Result<()> {
    let mut head = String::with_capacity(256);
    head.push_str(&format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        reason(response.status)
    ));
    head.push_str("Connection: close\r\n");
    head.push_str("X-Content-Type-Options: nosniff\r\n");
    head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    writer
        .write_all(head.as_bytes())
        .map_err(|err| Error::invalid(format!("the response could not be written: {err}")))?;
    writer
        .write_all(&response.body)
        .map_err(|err| Error::invalid(format!("the response could not be written: {err}")))?;
    writer
        .flush()
        .map_err(|err| Error::invalid(format!("the response could not be flushed: {err}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn reads_a_simple_request() {
        let text = "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:8080\r\n\r\n";
        let mut reader = BufReader::new(text.as_bytes());
        let request = read_request(&mut reader).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path(), "/api/state");
        assert_eq!(request.query(), "");
        assert_eq!(request.header("host"), Some("127.0.0.1:8080"));
        assert!(request.body.is_empty());
    }

    #[test]
    fn reads_a_query_string() {
        let text = "GET /api/region?x=1&y=2 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut reader = BufReader::new(text.as_bytes());
        let request = read_request(&mut reader).unwrap();
        assert_eq!(request.path(), "/api/region");
        assert_eq!(request.query(), "x=1&y=2");
    }

    #[test]
    fn reads_a_body() {
        let text = "POST /api/fill HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello";
        let mut reader = BufReader::new(text.as_bytes());
        let request = read_request(&mut reader).unwrap();
        assert_eq!(request.body_text().unwrap(), "hello");
    }

    #[test]
    fn refuses_a_chunked_body() {
        let text = "POST /api/fill HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n";
        let mut reader = BufReader::new(text.as_bytes());
        assert!(read_request(&mut reader).is_err());
    }

    #[test]
    fn refuses_a_body_that_is_too_large() {
        let text = format!(
            "POST /api/fill HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        let mut reader = BufReader::new(text.as_bytes());
        assert!(read_request(&mut reader).is_err());
    }

    #[test]
    fn refuses_a_head_that_is_too_large() {
        let mut text = String::from("GET / HTTP/1.1\r\n");
        for index in 0..2000 {
            text.push_str(&format!("X-Filler-{index}: {}\r\n", "a".repeat(40)));
        }
        text.push_str("\r\n");
        let mut reader = BufReader::new(text.as_bytes());
        assert!(read_request(&mut reader).is_err());
    }

    #[test]
    fn writes_a_response() {
        let response = Response::new(200, "text/plain", b"hello".to_vec());
        let mut out = Vec::new();
        write_response(&mut out, &response).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("Content-Length: 5\r\n"));
        assert!(text.ends_with("\r\n\r\nhello"));
    }
}
