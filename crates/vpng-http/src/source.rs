//! The HTTP source: a VPNG container fetched over HTTP Range requests.
//!
//! The format already supports this. Every tile payload has its own offset
//! and length in the directory, so a reader over the network can fetch the
//! 64-byte header, the directory, and then exactly the payloads it needs.
//! **No format change is needed, and none is made.**
//!
//! # What the source guarantees
//!
//! * A range request asks for exactly the bytes the reader wants. A server
//!   that ignores the `Range` header and replies `200 OK` with the whole
//!   object is **refused**, not silently accepted: the reader would then
//!   download the entire file and call it a range request.
//! * The `Content-Range` header of every `206 Partial Content` reply is
//!   checked against the range that was asked for. A server that answers a
//!   different range is refused.
//! * A short body is an error, never a short buffer. The reader must never
//!   be handed half a payload and have to guess.
//!
//! # The accounting
//!
//! The source counts the requests it made and the bytes it received, so a
//! report can state the network volume rather than guess at it. Fetching one
//! tile of a large image costs one request and that tile's bytes — not the
//! whole file.

use std::fmt;
use std::time::Duration;

use ureq::{Agent, Error as UreqError};
use vpng_core::{Error, Result};
use vpng_source::ByteSource;

/// An I/O failure, carrying the message the reader will show.
fn io_error(message: impl Into<String>) -> Error {
    Error::Io(std::io::Error::other(message.into()))
}

/// How an [`HttpSource`] behaves.
#[derive(Debug, Clone)]
pub struct HttpSourceOptions {
    /// The `User-Agent` sent with every request.
    pub user_agent: String,
    /// The whole-request timeout, including reading the body.
    pub timeout: Option<Duration>,
    /// The connect timeout.
    pub connect_timeout: Option<Duration>,
    /// The largest number of bytes one range request may return.
    ///
    /// A payload larger than this is fetched in several requests, so a
    /// single response never grows without bound.
    pub max_range_bytes: u64,
}

impl Default for HttpSourceOptions {
    fn default() -> Self {
        HttpSourceOptions {
            user_agent: concat!("vpng-http/", env!("CARGO_PKG_VERSION")).to_string(),
            timeout: Some(Duration::from_secs(60)),
            connect_timeout: Some(Duration::from_secs(20)),
            max_range_bytes: 8 * 1024 * 1024,
        }
    }
}

/// What the source knows about the object it is reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpSourceInfo {
    /// The object's length in bytes.
    pub length: u64,
    /// The number of HTTP requests made so far.
    pub requests: u64,
    /// The number of body bytes received so far.
    pub bytes_received: u64,
}

/// A VPNG container fetched over HTTP Range requests.
///
/// The source owns an HTTP agent and the URL. It learns the object's length
/// when it is opened and remembers it, so the header, the directory and
/// every tile payload are fetched with the byte ranges they actually
/// occupy.
pub struct HttpSource {
    url: String,
    agent: Agent,
    options: HttpSourceOptions,
    length: u64,
    requests: u64,
    bytes_received: u64,
}

impl fmt::Debug for HttpSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpSource")
            .field("url", &self.url)
            .field("length", &self.length)
            .field("requests", &self.requests)
            .field("bytes_received", &self.bytes_received)
            .finish()
    }
}

impl HttpSource {
    /// Open a URL and learn how long the object is.
    ///
    /// The length is learned with a `HEAD` request when the server allows
    /// one, and otherwise with a one-byte range request. Either way the
    /// source refuses to continue unless it can fetch **ranges**: a server
    /// that will not answer a range request is not a usable source for a
    /// random-access reader, and saying so now is better than downloading
    /// the whole object later.
    pub fn open(url: &str, options: HttpSourceOptions) -> Result<Self> {
        if url.is_empty() {
            return Err(Error::invalid("the URL is empty"));
        }
        if options.max_range_bytes == 0 {
            return Err(Error::invalid("the maximum range size must be non-zero"));
        }

        let agent = build_agent(&options)?;
        let mut source = HttpSource {
            url: url.to_string(),
            agent,
            options,
            length: 0,
            requests: 0,
            bytes_received: 0,
        };
        source.length = source.probe_length()?;
        Ok(source)
    }

    /// The URL the source reads from.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The object's length in bytes.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// What the source knows about the object.
    pub fn info(&self) -> HttpSourceInfo {
        HttpSourceInfo {
            length: self.length,
            requests: self.requests,
            bytes_received: self.bytes_received,
        }
    }

    /// Learn the object's length and prove that ranges work.
    ///
    /// `HEAD` is tried first because it costs no body bytes, but the answer
    /// is never taken on trust: a one-byte range request follows, which both
    /// proves the server honours `Range` and carries the authoritative
    /// length in its `Content-Range` header. A server that will not answer
    /// a range request is refused **here**, before the caller spends a
    /// request on a whole-object download.
    fn probe_length(&mut self) -> Result<u64> {
        let head_length = self.probe_head_length()?;

        let (bytes, total) = self.fetch_range(0, 1)?;
        if bytes.len() != 1 {
            return Err(io_error(format!(
                "`{}` answered a one-byte range request with {} bytes",
                self.url,
                bytes.len()
            )));
        }

        if let Some(head_length) = head_length {
            if head_length != total {
                return Err(Error::corrupt(format!(
                    "`{}` reported a length of {head_length} with HEAD and {total} with a range request; the object changed while it was being opened",
                    self.url
                )));
            }
        }
        Ok(total)
    }

    /// Ask the server for the object's length with a `HEAD` request.
    ///
    /// A server that refuses `HEAD` is not an error: the range probe that
    /// follows answers the same question.
    fn probe_head_length(&mut self) -> Result<Option<u64>> {
        let head = self.agent.head(&self.url).call();
        match head {
            Ok(response) => {
                self.requests += 1;
                let status = response.status().as_u16();
                if status == 200 {
                    Ok(content_length(response.headers()))
                } else {
                    Ok(None)
                }
            }
            Err(UreqError::StatusCode(code)) if code == 405 || code == 501 => {
                self.requests += 1;
                Ok(None)
            }
            Err(UreqError::StatusCode(code)) => {
                self.requests += 1;
                Err(io_error(format!(
                    "the server answered HEAD on `{}` with status {code}",
                    self.url
                )))
            }
            Err(error) => {
                self.requests += 1;
                Err(io_error(format!("cannot reach `{}`: {error}", self.url)))
            }
        }
    }

    /// Fetch one range and return the bytes and the object's total length.
    fn fetch_range(&mut self, offset: u64, len: u64) -> Result<(Vec<u8>, u64)> {
        let end = offset
            .checked_add(len)
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| Error::invalid(format!("the byte range {offset}..{len} overflows")))?;

        let response = self
            .agent
            .get(&self.url)
            .header("Range", format!("bytes={offset}-{end}"))
            .call()
            .map_err(|error| {
                io_error(format!(
                    "the range request for bytes {offset}-{end} of `{}` failed: {error}",
                    self.url
                ))
            })?;
        self.requests += 1;

        let status = response.status().as_u16();
        if status != 206 {
            // 200 means the server ignored the Range header and is about to
            // send the whole object. That is not a range request, and
            // accepting it would turn a random-access reader into a
            // whole-file download.
            return Err(Error::unsupported(format!(
                "`{}` answered a range request with status {status}; the server does not honour HTTP Range requests",
                self.url
            )));
        }

        let (first, last, total) = parse_content_range(response.headers()).ok_or_else(|| {
            Error::corrupt(format!(
                "`{}` answered a range request without a usable Content-Range header",
                self.url
            ))
        })?;
        if first != offset || last != end {
            return Err(Error::corrupt(format!(
                "`{}` answered the range {offset}-{end} with the range {first}-{last}",
                self.url
            )));
        }

        let mut body = response.into_body();
        let bytes = body.read_to_vec().map_err(|error| {
            io_error(format!(
                "cannot read the body of the range request for bytes {offset}-{end} of `{}`: {error}",
                self.url
            ))
        })?;
        self.bytes_received += bytes.len() as u64;

        if bytes.len() as u64 != len {
            return Err(Error::corrupt(format!(
                "`{}` answered the range {offset}-{end} with {} bytes",
                self.url,
                bytes.len()
            )));
        }

        Ok((bytes, total))
    }
}

impl ByteSource for HttpSource {
    fn length(&mut self) -> Result<u64> {
        Ok(self.length)
    }

    fn read_at(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let len = len as u64;
        if len == 0 {
            return Ok(Vec::new());
        }
        if offset
            .checked_add(len)
            .map(|end| end > self.length)
            .unwrap_or(true)
        {
            return Err(Error::invalid(format!(
                "the range {offset}..{} is outside the {} bytes the object has",
                offset.saturating_add(len),
                self.length
            )));
        }

        // A payload larger than the limit is fetched in several requests, so
        // no single response grows without bound.
        let mut out = Vec::with_capacity(vpng_core::checked_usize(len)?);
        let mut cursor = offset;
        let end = offset + len;
        while cursor < end {
            let take = self.options.max_range_bytes.min(end - cursor);
            let (bytes, _) = self.fetch_range(cursor, take)?;
            out.extend_from_slice(&bytes);
            cursor += take;
        }
        Ok(out)
    }

    fn fork(&self) -> Result<Self> {
        Ok(HttpSource {
            url: self.url.clone(),
            agent: build_agent(&self.options)?,
            options: self.options.clone(),
            length: self.length,
            requests: 0,
            bytes_received: 0,
        })
    }

    fn describe(&self) -> String {
        self.url.clone()
    }
}

/// Build the HTTP agent the source uses.
fn build_agent(options: &HttpSourceOptions) -> Result<Agent> {
    let builder = Agent::config_builder()
        .user_agent(options.user_agent.clone())
        // A 4xx or 5xx is turned into an error rather than a body, so the
        // reader never mistakes an error page for a tile payload.
        .http_status_as_error(true)
        .timeout_global(options.timeout)
        .timeout_connect(options.connect_timeout);
    Ok(builder.build().new_agent())
}

/// The `Content-Length` header, if it is present and is a number.
fn content_length(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

/// Parse a `Content-Range: bytes FIRST-LAST/TOTAL` header.
///
/// The total may be `*`, which means the server does not know the object's
/// length; that is reported as `None` and is an error for a container
/// reader, because the header's declared size must be checked against the
/// bytes actually present.
fn parse_content_range(headers: &ureq::http::HeaderMap) -> Option<(u64, u64, u64)> {
    let value = headers.get("content-range")?.to_str().ok()?;
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let total = total.trim().parse::<u64>().ok()?;
    let (first, last) = range.trim().split_once('-')?;
    let first = first.trim().parse::<u64>().ok()?;
    let last = last.trim().parse::<u64>().ok()?;
    if last < first {
        return None;
    }
    Some((first, last, total))
}
