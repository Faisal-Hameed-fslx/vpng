//! The local server the window talks to.
//!
//! # The trust model
//!
//! The editor has **no authentication** and it can write files. It is
//! therefore bound to the loopback interface and to nothing else, and it
//! checks two things before it answers a request that could change anything:
//!
//! * the **`Host` header** must name a loopback address, so a web page
//!   cannot reach the editor through a name that resolves to the machine
//!   (the DNS-rebinding attack);
//! * the **session token** must be presented in the `X-Vpng-Token` header,
//!   so a web page that guesses the address still cannot drive the editor.
//!
//! The token is printed in the URL the editor opens, and it comes from the
//! operating system's random source. The assets — the page, the script, the
//! stylesheet — are served without it, because they are static text and
//! carry no authority.
//!
//! # The concurrency
//!
//! Each connection is handled on its own thread, so the browser's several
//! requests do not queue behind one another. The editing session itself is
//! behind a mutex: one session, one document, and every command that touches
//! it is serialised.

use std::io::BufReader;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use vpng_core::{Error, ErrorKind, Result};
use vpng_json::Json;

use crate::assets;
use crate::http::{read_request, write_response, Request, Response};
use crate::protocol::{self, RectSpec};
use crate::session::{Colour, Session, Tool};

/// The header the session token travels in.
pub const TOKEN_HEADER: &str = "x-vpng-token";

/// The local server.
pub struct Server {
    listener: TcpListener,
    context: Context,
    stop: Arc<AtomicBool>,
    requests: Arc<AtomicU64>,
}

/// The shared state every request needs.
struct Context {
    session: Arc<Mutex<Session>>,
    token: String,
}

impl Context {
    /// The editing session.
    fn lock(&self) -> Result<MutexGuard<'_, Session>> {
        self.session
            .lock()
            .map_err(|_| Error::internal("the editing session's lock was poisoned"))
    }
}

impl Server {
    /// Bind a server to an address and open it over a session.
    pub fn bind(address: SocketAddr, session: Session, token: String) -> Result<Self> {
        let listener = TcpListener::bind(address).map_err(|err| {
            Error::invalid(format!("the editor could not listen on {address}: {err}"))
        })?;
        listener.set_nonblocking(true).map_err(|err| {
            Error::invalid(format!(
                "the editor could not set its socket non-blocking: {err}"
            ))
        })?;
        Ok(Server {
            listener,
            context: Context {
                session: Arc::new(Mutex::new(session)),
                token,
            },
            stop: Arc::new(AtomicBool::new(false)),
            requests: Arc::new(AtomicU64::new(0)),
        })
    }

    /// The address the server listens on.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .map_err(|err| Error::internal(format!("the editor lost its address: {err}")))
    }

    /// The session token the window must present.
    pub fn token(&self) -> &str {
        &self.context.token
    }

    /// The number of requests the server has answered.
    pub fn requests(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Ask the server to stop.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// The address the window should open.
    pub fn url(&self) -> Result<String> {
        let address = self.local_addr()?;
        Ok(format!("http://{address}/?token={}", self.context.token))
    }

    /// Answer requests until the server is told to stop.
    pub fn serve(&self) -> Result<()> {
        while !self.stop.load(Ordering::Relaxed) {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let context = Context {
                        session: Arc::clone(&self.context.session),
                        token: self.context.token.clone(),
                    };
                    let requests = Arc::clone(&self.requests);
                    thread::spawn(move || {
                        handle_connection(stream, context, requests);
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => {
                    return Err(Error::invalid(format!(
                        "the editor could not accept a connection: {err}"
                    )))
                }
            }
        }
        Ok(())
    }

    /// Answer one request.
    pub fn dispatch(&self, request: &Request) -> Response {
        self.requests.fetch_add(1, Ordering::Relaxed);
        dispatch(&self.context, request)
    }
}

/// Answer one request.
fn dispatch(context: &Context, request: &Request) -> Response {
    if !host_is_loopback(request.header("host")) {
        return error_response(&Error::unsupported(
            "the editor answers only requests that name a loopback address",
        ));
    }

    let path = request.path();
    if path.starts_with("/api/") {
        if request.header(TOKEN_HEADER) != Some(context.token.as_str()) {
            return error_response(&Error::unsupported(
                "the request carries no session token, so it is refused",
            ));
        }
        return api(context, request, path);
    }

    match assets::asset(path) {
        Some((content_type, text)) => Response::new(200, content_type, text.as_bytes().to_vec()),
        None => error_response(&Error::invalid(format!("there is no `{path}`"))),
    }
}

/// Answer one API request.
fn api(context: &Context, request: &Request, path: &str) -> Response {
    let outcome: Result<Response> = (|| match (request.method.as_str(), path) {
        ("GET", "/api/state") => {
            let session = context.lock()?;
            Ok(Response::json(200, &protocol::state_json(&session.info())))
        }
        ("GET", "/api/ping") => Ok(Response::json(
            200,
            &Json::Object(vec![("pong".into(), Json::Bool(true))]),
        )),
        ("POST", "/api/region") => region(context, request),
        ("POST", "/api/pick") => pick(context, request),
        ("POST", "/api/fill") => fill(context, request),
        ("POST", "/api/alpha") => alpha(context, request),
        ("POST", "/api/tool") => tool(context, request),
        ("POST", "/api/colour") => colour(context, request),
        ("POST", "/api/undo") => undo(context),
        ("POST", "/api/redo") => redo(context),
        ("POST", "/api/save") => save(context, request),
        (_, _) => Err(Error::invalid(format!("there is no `{path}`"))),
    })();
    match outcome {
        Ok(response) => response,
        Err(error) => error_response(&error),
    }
}

/// Read a rectangle of the picture for the display.
fn region(context: &Context, request: &Request) -> Result<Response> {
    let body = body_json(request)?;
    let rect = RectSpec::from_json(&body)?.to_rect()?;
    let mut session = context.lock()?;
    let pixels = session.region(rect)?;
    let info = session.info();
    Ok(Response::new(200, "application/octet-stream", pixels)
        .with_header("X-Vpng-Width", rect.width)
        .with_header("X-Vpng-Height", rect.height)
        .with_header("X-Vpng-Format", crate::session::format_name(info.format))
        .with_header("X-Vpng-Visible-Tiles", info.plan.visible_tiles)
        .with_header("X-Vpng-Prefetch-Tiles", info.plan.prefetch_tiles))
}

/// Read one pixel.
fn pick(context: &Context, request: &Request) -> Result<Response> {
    let body = body_json(request)?;
    let x = body
        .get("x")
        .and_then(Json::as_u32)
        .ok_or_else(|| Error::invalid("the request has no `x` field"))?;
    let y = body
        .get("y")
        .and_then(Json::as_u32)
        .ok_or_else(|| Error::invalid("the request has no `y` field"))?;
    let mut session = context.lock()?;
    let pixel = session.pick(x, y)?;
    Ok(Response::json(200, &protocol::pixel_json(&pixel)))
}

/// Fill a rectangle with the current colour.
fn fill(context: &Context, request: &Request) -> Result<Response> {
    let body = body_json(request)?;
    let rect = RectSpec::from_json(&body)?.to_rect()?;
    let mut session = context.lock()?;
    let report = session.fill(rect)?;
    Ok(Response::json(200, &protocol::edit_json(&report)))
}

/// Set the alpha channel over a rectangle.
fn alpha(context: &Context, request: &Request) -> Result<Response> {
    let body = body_json(request)?;
    let rect = RectSpec::from_json(&body)?.to_rect()?;
    let value = body
        .get("value")
        .and_then(Json::as_u32)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| Error::invalid("the request has no `value` field between 0 and 255"))?;
    let mut session = context.lock()?;
    let report = session.set_alpha(rect, value)?;
    Ok(Response::json(200, &protocol::edit_json(&report)))
}

/// Choose the active tool.
fn tool(context: &Context, request: &Request) -> Result<Response> {
    let body = body_json(request)?;
    let tool: Tool = protocol::tool_from_json(&body)?;
    let mut session = context.lock()?;
    session.set_tool(tool);
    Ok(Response::json(200, &protocol::state_json(&session.info())))
}

/// Choose the current colour.
fn colour(context: &Context, request: &Request) -> Result<Response> {
    let body = body_json(request)?;
    let colour: Colour = protocol::colour_from_json(&body)?;
    let mut session = context.lock()?;
    session.set_colour(colour);
    Ok(Response::json(200, &protocol::state_json(&session.info())))
}

/// Undo the newest edit.
fn undo(context: &Context) -> Result<Response> {
    let mut session = context.lock()?;
    let undone = session.undo()?;
    Ok(Response::json(
        200,
        &Json::Object(vec![
            ("undone".into(), Json::Bool(undone)),
            ("state".into(), protocol::state_json(&session.info())),
        ]),
    ))
}

/// Redo the newest undone edit.
fn redo(context: &Context) -> Result<Response> {
    let mut session = context.lock()?;
    let redone = session.redo()?;
    Ok(Response::json(
        200,
        &Json::Object(vec![
            ("redone".into(), Json::Bool(redone)),
            ("state".into(), protocol::state_json(&session.info())),
        ]),
    ))
}

/// Save the document.
fn save(context: &Context, request: &Request) -> Result<Response> {
    let body = body_json(request)?;
    let path = protocol::path_from_json(&body)?;
    let mut session = context.lock()?;
    let report = session.save(path.as_deref().map(std::path::Path::new))?;
    Ok(Response::json(
        200,
        &Json::Object(vec![
            (
                "path".into(),
                Json::from(report.path.to_string_lossy().into_owned()),
            ),
            ("file_size".into(), Json::from(report.file_size)),
            ("tiles_rewritten".into(), Json::from(report.tiles_rewritten)),
            ("tiles_copied".into(), Json::from(report.tiles_copied)),
            (
                "copied_payload_bytes".into(),
                Json::from(report.copied_payload_bytes),
            ),
            (
                "rewritten_payload_bytes".into(),
                Json::from(report.rewritten_payload_bytes),
            ),
            (
                "baseline_advanced".into(),
                Json::from(report.baseline_advanced),
            ),
            (
                "preview_levels_dropped".into(),
                Json::from(report.preview_levels_dropped),
            ),
            (
                "elapsed_ms".into(),
                Json::from(report.elapsed.as_millis() as u64),
            ),
        ]),
    ))
}

/// Read the body of a request as JSON.
fn body_json(request: &Request) -> Result<Json> {
    let text = request.body_text()?;
    if text.trim().is_empty() {
        return Ok(Json::Object(Vec::new()));
    }
    vpng_json::parse(&text)
}

/// Turn an error into a response.
fn error_response(error: &Error) -> Response {
    Response::json(status_for(error.kind()), &protocol::error_json(error))
}

/// The HTTP status an error kind is answered with.
fn status_for(kind: ErrorKind) -> u16 {
    match kind {
        ErrorKind::InvalidArgument | ErrorKind::UnsupportedFeature => 400,
        ErrorKind::CorruptFile => 422,
        ErrorKind::ResourceLimit | ErrorKind::SizeLimitExceeded => 413,
        ErrorKind::VerificationFailed => 409,
        ErrorKind::Io | ErrorKind::Internal => 500,
    }
}

/// Whether a `Host` header names a loopback address.
///
/// A request without a `Host` header is allowed: the browser always sends
/// one, and the threat the check answers is a *named* host that resolves to
/// the machine.
pub fn host_is_loopback(host: Option<&str>) -> bool {
    let Some(host) = host else {
        return true;
    };
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        // An IPv6 literal: `[::1]:8080`.
        rest.split(']').next().unwrap_or("")
    } else if host.matches(':').count() == 1 {
        // A name and a port: `127.0.0.1:8080`.
        host.split(':').next().unwrap_or("")
    } else {
        host
    };
    name == "127.0.0.1" || name == "localhost" || name == "::1"
}

/// Handle one connection.
fn handle_connection(stream: TcpStream, context: Context, requests: Arc<AtomicU64>) {
    let mut writer = match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);

    let response = match read_request(&mut reader) {
        Ok(request) => {
            requests.fetch_add(1, Ordering::Relaxed);
            dispatch(&context, &request)
        }
        Err(error) => error_response(&error),
    };

    let _ = write_response(&mut writer, &response);

    // Close the connection cleanly. Windows turns a close that happens while
    // data is still in flight into an *abort*, and the client then sees
    // `ConnectionAborted` instead of the response it was sent. Shutting the
    // sending side down first sends a FIN after the last byte, so the
    // client reads the whole response and then sees the end of the stream.
    let _ = writer.shutdown(std::net::Shutdown::Write);
}

/// A session token, from the operating system's random source.
///
/// The standard library's `RandomState` is seeded by the operating system's
/// random number generator, so four of its hashes are four blocks of
/// unpredictable bytes. The token is 64 hexadecimal characters.
pub fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut token = String::with_capacity(32);
    for index in 0..4u64 {
        let state = RandomState::new();
        let mut hasher = state.build_hasher();
        hasher.write_u64(index);
        hasher.write_u64(std::process::id() as u64);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0),
        );
        token.push_str(&format!("{:016x}", hasher.finish()));
    }
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_loopback_host() {
        assert!(host_is_loopback(Some("127.0.0.1:8080")));
        assert!(host_is_loopback(Some("localhost:8080")));
        assert!(host_is_loopback(Some("localhost")));
        assert!(host_is_loopback(Some("[::1]:8080")));
        assert!(host_is_loopback(None));
    }

    #[test]
    fn refuses_a_named_host() {
        assert!(!host_is_loopback(Some("example.invalid")));
        assert!(!host_is_loopback(Some("example.invalid:8080")));
        assert!(!host_is_loopback(Some("192.168.1.10:8080")));
    }

    #[test]
    fn the_token_is_long_and_different_each_time() {
        let first = random_token();
        let second = random_token();
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}
