//! The editor's whole surface, driven over HTTP.
//!
//! These tests are the editor's acceptance tests. They open a real file,
//! talk to the real server with a real HTTP client, and check that the
//! picture that comes back is the picture that was asked for — and that the
//! file on disk is still a valid VPNG file afterwards.
//!
//! The editor needs no display server, so the whole of it is exercised here
//! and in the continuous-integration job.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use common::{gradient, scratch, write_picture};
use vpng_core::{Limits, PixelFormat};
use vpng_json::Json;
use vpng_ui::{random_token, Server, Session};

/// A running editor and the address it answers on.
struct Editor {
    server: Arc<Server>,
    agent: ureq::Agent,
    url: String,
    token: String,
    path: std::path::PathBuf,
}

impl Editor {
    /// Open an editor over a new picture.
    fn start(name: &str, format: PixelFormat) -> Editor {
        let directory = scratch(name);
        let path = write_picture(&directory, "picture.vpng", &gradient(64, 48, format));
        let session = Session::open(&path, &Limits::default()).unwrap();
        let token = random_token();
        let server =
            Arc::new(Server::bind("127.0.0.1:0".parse().unwrap(), session, token.clone()).unwrap());
        let url = server.url().unwrap();
        let worker = Arc::clone(&server);
        std::thread::spawn(move || {
            let _ = worker.serve();
        });

        // The editor closes every connection, so the client must not pool
        // one: a pooled connection that the server has already closed shows
        // up as an aborted connection rather than as a clean end of stream.
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_idle_connections(0)
            .build();
        Editor {
            server,
            agent: config.into(),
            url,
            token,
            path,
        }
    }

    /// The address of an API route.
    fn route(&self, path: &str) -> String {
        let base = self.url.split("?").next().unwrap().trim_end_matches('/');
        format!("{base}{path}")
    }

    /// A GET with the token.
    fn get(&self, path: &str) -> (u16, Vec<u8>) {
        let mut response = self
            .agent
            .get(self.route(path))
            .header("X-Vpng-Token", &self.token)
            .call()
            .expect("the editor answers");
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_vec().expect("the body reads");
        (status, body)
    }

    /// A POST with the token and a JSON body.
    fn post(&self, path: &str, body: &str) -> (u16, Vec<u8>) {
        let mut response = self
            .agent
            .post(self.route(path))
            .header("X-Vpng-Token", &self.token)
            .header("Content-Type", "application/json")
            .send(body)
            .expect("the editor answers");
        let status = response.status().as_u16();
        let bytes = response.body_mut().read_to_vec().expect("the body reads");
        (status, bytes)
    }

    /// A POST without the token.
    fn post_without_a_token(&self, path: &str, body: &str) -> u16 {
        self.agent
            .post(self.route(path))
            .header("Content-Type", "application/json")
            .send(body)
            .expect("the editor answers")
            .status()
            .as_u16()
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        self.server.stop();
    }
}

#[test]
fn the_editor_serves_its_window() {
    let editor = Editor::start("window", PixelFormat::rgb8());
    let (status, body) = editor.get("/");
    assert_eq!(status, 200);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("VPNG editor"), "the page has no title");
    assert!(text.contains("/app.js"), "the page loads no script");

    let (status, body) = editor.get("/app.js");
    assert_eq!(status, 200);
    assert!(String::from_utf8(body).unwrap().contains("canvas"));

    let (status, _) = editor.get("/geometry.js");
    assert_eq!(status, 200);

    let (status, _) = editor.get("/style.css");
    assert_eq!(status, 200);
}

#[test]
fn the_editor_refuses_an_unknown_path() {
    let editor = Editor::start("unknown", PixelFormat::rgb8());
    let (status, _) = editor.get("/nowhere");
    assert_eq!(status, 400);
}

#[test]
fn the_editor_refuses_a_request_without_the_token() {
    let editor = Editor::start("token", PixelFormat::rgb8());
    let status = editor.post_without_a_token("/api/state", "{}");
    assert_eq!(status, 400);
}

#[test]
fn the_editor_refuses_a_request_that_names_another_host() {
    let editor = Editor::start("host", PixelFormat::rgb8());
    let address = editor.server.local_addr().unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    let request = format!(
        "GET /api/state HTTP/1.1\r\nHost: example.invalid\r\nX-Vpng-Token: {}\r\n\r\n",
        editor.token
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut text = String::new();
    stream.read_to_string(&mut text).unwrap();
    assert!(
        text.starts_with("HTTP/1.1 400"),
        "the editor answered: {text}"
    );
}

#[test]
fn the_editor_reports_the_state() {
    let editor = Editor::start("state", PixelFormat::rgb8());
    let (status, body) = editor.get("/api/state");
    assert_eq!(status, 200);
    let state = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(state.get("width").and_then(Json::as_u32), Some(64));
    assert_eq!(state.get("height").and_then(Json::as_u32), Some(48));
    assert_eq!(state.get("format").and_then(Json::as_str), Some("rgb8"));
}

#[test]
fn the_editor_serves_a_region() {
    let editor = Editor::start("region", PixelFormat::rgb8());
    let (status, body) = editor.post(
        "/api/region",
        r#"{"x": 0, "y": 0, "width": 4, "height": 3}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(body.len(), 4 * 3 * 4);

    // The first pixel of the gradient is black, and the second is one step
    // along the red channel.
    assert_eq!(&body[0..4], &[0, 0, 0, 255]);
    assert_eq!(&body[4..8], &[1, 0, 1, 255]);
}

#[test]
fn the_editor_refuses_a_region_it_cannot_serve() {
    let editor = Editor::start("too-big", PixelFormat::rgb8());
    let (status, body) = editor.post(
        "/api/region",
        r#"{"x": 0, "y": 0, "width": 64, "height": 48}"#,
    );
    // The picture is 64x48, which is inside the picture but past the limit
    // of four million pixels? No: 3072 pixels is well inside it. The region
    // is therefore served.
    assert_eq!(status, 200);
    assert_eq!(body.len(), 64 * 48 * 4);

    // A rectangle outside the picture is refused.
    let (status, _) = editor.post(
        "/api/region",
        r#"{"x": 60, "y": 0, "width": 10, "height": 3}"#,
    );
    assert_eq!(status, 400);
}

#[test]
fn the_editor_picks_a_pixel() {
    let editor = Editor::start("pick", PixelFormat::rgb8());
    let (status, body) = editor.post("/api/pick", r#"{"x": 5, "y": 7}"#);
    assert_eq!(status, 200);
    let pixel = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(pixel.get("r").and_then(Json::as_u32), Some(5));
    assert_eq!(pixel.get("g").and_then(Json::as_u32), Some(7));
    assert_eq!(pixel.get("b").and_then(Json::as_u32), Some(12));
    assert_eq!(pixel.get("a").and_then(Json::as_u32), Some(255));
}

#[test]
fn the_editor_fills_saves_and_keeps_the_picture_exact() {
    let editor = Editor::start("fill", PixelFormat::rgb8());

    let (status, _) = editor.post("/api/colour", r#"{"r": 9, "g": 8, "b": 7, "a": 255}"#);
    assert_eq!(status, 200);

    let (status, body) = editor.post("/api/fill", r#"{"x": 2, "y": 3, "width": 4, "height": 5}"#);
    assert_eq!(status, 200);
    let report = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(report.get("pixels").and_then(Json::as_u64), Some(20));

    // The state says the document is dirty.
    let (_, body) = editor.get("/api/state");
    let state = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(state.get("dirty"), Some(&Json::Bool(true)));
    assert_eq!(state.get("dirty_tiles").and_then(Json::as_u32), Some(1));

    // The region shows the fill.
    let (_, body) = editor.post(
        "/api/region",
        r#"{"x": 0, "y": 0, "width": 8, "height": 8}"#,
    );
    assert_eq!(&body[(3 * 8 + 2) * 4..(3 * 8 + 2) * 4 + 4], &[9, 8, 7, 255]);

    // The save writes the file.
    let (status, body) = editor.post("/api/save", "{}");
    assert_eq!(status, 200);
    let saved = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(saved.get("tiles_rewritten").and_then(Json::as_u32), Some(1));

    // The file on disk is a valid container that holds the edited picture.
    let bytes = std::fs::read(&editor.path).unwrap();
    let document = vpng_decoder::Document::parse(&bytes, &Limits::default()).unwrap();
    let decoded = document.decode_all(&Limits::default()).unwrap();
    let pixels = vpng_ui::to_rgba8(&decoded).unwrap();
    assert_eq!(
        &pixels[(3 * 64 + 2) * 4..(3 * 64 + 2) * 4 + 4],
        &[9, 8, 7, 255]
    );
    assert_eq!(&pixels[0..4], &[0, 0, 0, 255]);
}

#[test]
fn the_editor_undoes_and_redoes() {
    let editor = Editor::start("undo", PixelFormat::rgb8());
    editor.post("/api/colour", r#"{"r": 9, "g": 8, "b": 7, "a": 255}"#);
    editor.post("/api/fill", r#"{"x": 0, "y": 0, "width": 4, "height": 4}"#);

    let (status, body) = editor.post("/api/undo", "{}");
    assert_eq!(status, 200);
    let undone = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(undone.get("undone"), Some(&Json::Bool(true)));

    let (_, body) = editor.post(
        "/api/region",
        r#"{"x": 0, "y": 0, "width": 4, "height": 4}"#,
    );
    assert_eq!(&body[0..4], &[0, 0, 0, 255]);

    let (status, body) = editor.post("/api/redo", "{}");
    assert_eq!(status, 200);
    let redone = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(redone.get("redone"), Some(&Json::Bool(true)));

    let (_, body) = editor.post(
        "/api/region",
        r#"{"x": 0, "y": 0, "width": 4, "height": 4}"#,
    );
    assert_eq!(&body[0..4], &[9, 8, 7, 255]);
}

#[test]
fn the_editor_refuses_an_alpha_edit_on_a_picture_without_alpha() {
    let editor = Editor::start("alpha", PixelFormat::rgb8());
    let (status, body) = editor.post(
        "/api/alpha",
        r#"{"x": 0, "y": 0, "width": 4, "height": 4, "value": 128}"#,
    );
    assert_eq!(status, 400);
    let error = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    let inner = error.get("error").unwrap();
    assert_eq!(
        inner.get("kind").and_then(Json::as_str),
        Some("unsupported-feature")
    );
    assert!(inner
        .get("message")
        .and_then(Json::as_str)
        .unwrap()
        .contains("alpha"));
}

#[test]
fn the_editor_sets_the_alpha_channel() {
    let editor = Editor::start("alpha-ok", PixelFormat::rgba8());
    let (status, _) = editor.post(
        "/api/alpha",
        r#"{"x": 0, "y": 0, "width": 4, "height": 4, "value": 128}"#,
    );
    assert_eq!(status, 200);
    let (_, body) = editor.post(
        "/api/region",
        r#"{"x": 0, "y": 0, "width": 4, "height": 4}"#,
    );
    assert_eq!(body[3], 128);
}

#[test]
fn the_editor_chooses_a_tool() {
    let editor = Editor::start("tool", PixelFormat::rgb8());
    let (status, body) = editor.post("/api/tool", r#"{"tool": "fill"}"#);
    assert_eq!(status, 200);
    let state = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(state.get("tool").and_then(Json::as_str), Some("fill"));

    let (status, _) = editor.post("/api/tool", r#"{"tool": "teleport"}"#);
    assert_eq!(status, 400);
}

#[test]
fn the_editor_counts_its_requests() {
    let editor = Editor::start("count", PixelFormat::rgb8());
    let before = editor.server.requests();
    editor.get("/api/state");
    editor.get("/api/state");
    assert_eq!(editor.server.requests(), before + 2);
}
