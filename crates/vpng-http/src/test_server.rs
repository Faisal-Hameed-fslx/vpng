//! A minimal HTTP/1.1 server, used by the tests only.
//!
//! The server exists so the HTTP source can be tested against a server it
//! does not control: one that honours `Range`, one that ignores it, one
//! that answers with the wrong range, and one that truncates the body.
//! Every answer is counted, so a test can state how many requests the
//! reader made — the number that matters for a random-access reader.
//!
//! The module is compiled only under `cfg(test)` or with the
//! `test-support` feature, so it never reaches a release build of the crate.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// How the server answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// Honour `Range` requests with `206 Partial Content`.
    Ranges,
    /// Ignore the `Range` header and always answer `200 OK` with the whole
    /// object, which is what a server that does not support ranges does.
    IgnoreRanges,
    /// Answer a range request with a `Content-Range` that names a
    /// different range than the one asked for.
    WrongRange,
    /// Answer a range request with a body shorter than the range.
    ShortBody,
    /// Refuse `HEAD` with `405 Method Not Allowed`.
    NoHead,
}

/// A server that serves one fixed body over HTTP.
pub struct TestServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    range_requests: Arc<AtomicUsize>,
    bytes_sent: Arc<AtomicUsize>,
}

impl TestServer {
    /// Start a server that serves `body` with the given behaviour.
    pub fn start(body: Vec<u8>, behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the test server can bind a port");
        let address = listener
            .local_addr()
            .expect("the bound socket has an address");

        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let range_requests = Arc::new(AtomicUsize::new(0));
        let bytes_sent = Arc::new(AtomicUsize::new(0));

        let thread_stop = Arc::clone(&stop);
        let thread_requests = Arc::clone(&requests);
        let thread_ranges = Arc::clone(&range_requests);
        let thread_bytes = Arc::clone(&bytes_sent);
        let body = Arc::new(body);

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { break };
                let body = Arc::clone(&body);
                let requests = Arc::clone(&thread_requests);
                let ranges = Arc::clone(&thread_ranges);
                let bytes = Arc::clone(&thread_bytes);
                std::thread::spawn(move || {
                    let _ = serve(stream, &body, behaviour, &requests, &ranges, &bytes);
                });
            }
        });

        TestServer {
            address,
            stop,
            requests,
            range_requests,
            bytes_sent,
        }
    }

    /// The URL the object is served at.
    pub fn url(&self) -> String {
        format!("http://{}/object.vpng", self.address)
    }

    /// The number of HTTP requests the server answered.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// The number of requests that carried a `Range` header.
    pub fn range_requests(&self) -> usize {
        self.range_requests.load(Ordering::SeqCst)
    }

    /// The number of body bytes the server sent.
    pub fn bytes_sent(&self) -> usize {
        self.bytes_sent.load(Ordering::SeqCst)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Unblock the accept loop with one throwaway connection.
        let _ = TcpStream::connect(self.address);
    }
}

/// Answer one request.
fn serve(
    mut stream: TcpStream,
    body: &[u8],
    behaviour: Behaviour,
    requests: &AtomicUsize,
    range_requests: &AtomicUsize,
    bytes_sent: &AtomicUsize,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    requests.fetch_add(1, Ordering::SeqCst);

    let mut range = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_string());
            }
        }
    }

    let is_head = request_line.starts_with("HEAD");
    if is_head && behaviour == Behaviour::NoHead {
        write_response(&mut stream, "405 Method Not Allowed", &[], None, None)?;
        return Ok(());
    }

    let total = body.len() as u64;
    let asked = range.as_ref().and_then(|value| parse_range(value));

    if is_head {
        write_response(&mut stream, "200 OK", &[], None, Some(total))?;
        return Ok(());
    }

    match (behaviour, asked) {
        (Behaviour::IgnoreRanges, _) | (_, None) => {
            bytes_sent.fetch_add(body.len(), Ordering::SeqCst);
            write_response(&mut stream, "200 OK", body, None, Some(total))?;
        }
        (Behaviour::WrongRange, Some((first, last))) => {
            range_requests.fetch_add(1, Ordering::SeqCst);
            let slice = slice_of(body, first, last);
            bytes_sent.fetch_add(slice.len(), Ordering::SeqCst);
            let wrong_first = first.saturating_sub(1);
            let wrong_last = last.saturating_sub(1);
            let header = format!("bytes {wrong_first}-{wrong_last}/{total}");
            write_response(
                &mut stream,
                "206 Partial Content",
                slice,
                Some(&header),
                Some(slice.len() as u64),
            )?;
        }
        (Behaviour::ShortBody, Some((first, last))) => {
            range_requests.fetch_add(1, Ordering::SeqCst);
            let slice = slice_of(body, first, last);
            // A one-byte range is answered in full: the truncation is for
            // the larger ranges, so the length probe still works.
            let short = if slice.len() > 1 {
                &slice[..slice.len() - 1]
            } else {
                slice
            };
            bytes_sent.fetch_add(short.len(), Ordering::SeqCst);
            let header = format!("bytes {first}-{last}/{total}");
            write_response(
                &mut stream,
                "206 Partial Content",
                short,
                Some(&header),
                Some(short.len() as u64),
            )?;
        }
        (Behaviour::Ranges, Some((first, last))) | (Behaviour::NoHead, Some((first, last))) => {
            range_requests.fetch_add(1, Ordering::SeqCst);
            let slice = slice_of(body, first, last);
            bytes_sent.fetch_add(slice.len(), Ordering::SeqCst);
            let header = format!("bytes {first}-{last}/{total}");
            write_response(
                &mut stream,
                "206 Partial Content",
                slice,
                Some(&header),
                Some(slice.len() as u64),
            )?;
        }
    }

    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

/// Write a response.
fn write_response(
    stream: &mut TcpStream,
    status: &str,
    body: &[u8],
    content_range: Option<&str>,
    length: Option<u64>,
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status}\r\nAccept-Ranges: bytes\r\n");
    if let Some(range) = content_range {
        head.push_str(&format!("Content-Range: {range}\r\n"));
    }
    if let Some(length) = length {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// The bytes of `body` in the inclusive range `first..=last`.
fn slice_of(body: &[u8], first: u64, last: u64) -> &[u8] {
    let start = (first as usize).min(body.len());
    let end = ((last as usize) + 1).min(body.len());
    &body[start..end]
}

/// Parse a `bytes=FIRST-LAST` header value.
fn parse_range(value: &str) -> Option<(u64, u64)> {
    let rest = value.trim().strip_prefix("bytes=")?;
    let (first, last) = rest.split_once('-')?;
    let first = first.trim().parse::<u64>().ok()?;
    let last = last.trim().parse::<u64>().ok()?;
    if last < first {
        return None;
    }
    Some((first, last))
}
