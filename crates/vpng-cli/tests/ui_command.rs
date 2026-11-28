//! The command line's graphical editor.
//!
//! The editor is a local program with no display server, so it is exercised
//! the way a user would exercise it: the command is run, the address it
//! prints is used, and the picture that comes back is checked against what
//! the file holds.
//!
//! The rules the command line must keep:
//!
//! * `vpng ui` opens a file and prints the editor's address;
//! * the address names the **loopback** interface and carries a token, so
//!   the editor cannot be driven from another machine;
//! * the editor serves the picture, an edit changes exactly the rectangle
//!   that was asked for, and a save writes a file that still verifies;
//! * a file the editing engine refuses is refused here too, with the
//!   engine's own reason.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use vpng_core::{Limits, PixelFormat};

/// The `vpng` binary, built by Cargo for this test.
const VPNG: &str = env!("CARGO_BIN_EXE_vpng");

/// A temporary directory for one test.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "vpng-cli-ui-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// A deterministic raw RGB8 image.
fn raw_pixels(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            pixels.push((x % 251) as u8);
            pixels.push((y % 241) as u8);
            pixels.push(((((x / 16) + (y / 16)) % 2) * 255) as u8);
        }
    }
    pixels
}

/// Encode a raw RGB8 image into a file.
fn make_picture(directory: &Path, width: u32, height: u32) -> PathBuf {
    let raw = directory.join("picture.raw");
    fs::write(&raw, raw_pixels(width, height)).unwrap();
    let vpng = directory.join("picture.vpng");
    let status = Command::new(VPNG)
        .args([
            "encode",
            raw.to_str().unwrap(),
            vpng.to_str().unwrap(),
            "--input-format",
            "raw",
            "--format",
            "rgb8",
            "--width",
            &width.to_string(),
            "--height",
            &height.to_string(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "the picture encodes");
    vpng
}

/// Read the editor's address from the lines it prints.
fn read_address(reader: &mut impl BufRead) -> String {
    let mut line = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "the editor printed no address");
        if let Some(rest) = line.trim().strip_prefix("vpng: the editor is at ") {
            return rest.to_string();
        }
    }
}

/// The editor, running as a child process.
struct Editor {
    child: Child,
    base: String,
    token: String,
}

impl Editor {
    /// Start the editor over a file and read the address it prints.
    fn start(picture: &Path) -> Editor {
        let mut child = Command::new(VPNG)
            .args(["ui", picture.to_str().unwrap(), "--no-browser"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);
        let address = read_address(&mut reader);

        let (base, token) = address
            .split_once("?token=")
            .expect("the address carries a token");
        Editor {
            child,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// A GET with the token.
    fn get(&self, path: &str) -> (u16, Vec<u8>) {
        let mut response = ureq::get(&format!("{}{path}", self.base))
            .header("X-Vpng-Token", &self.token)
            .call()
            .expect("the editor answers");
        let status = response.status().as_u16();
        let body = response.body_mut().read_to_vec().expect("the body reads");
        (status, body)
    }

    /// A POST with the token and a JSON body.
    fn post(&self, path: &str, body: &str) -> (u16, Vec<u8>) {
        let mut response = ureq::post(&format!("{}{path}", self.base))
            .header("X-Vpng-Token", &self.token)
            .header("Content-Type", "application/json")
            .send(body)
            .expect("the editor answers");
        let status = response.status().as_u16();
        let bytes = response.body_mut().read_to_vec().expect("the body reads");
        (status, bytes)
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn the_editor_opens_a_file_and_prints_its_address() {
    let directory = scratch("open");
    let picture = make_picture(&directory, 64, 48);
    let editor = Editor::start(&picture);

    assert!(
        editor.base.starts_with("http://127.0.0.1:"),
        "the editor is not on the loopback interface: {}",
        editor.base
    );
    assert_eq!(editor.token.len(), 64, "the token is too short");

    let (status, body) = editor.get("/api/state");
    assert_eq!(status, 200);
    let state = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(
        state.get("width").and_then(vpng_json::Json::as_u32),
        Some(64)
    );
    assert_eq!(
        state.get("height").and_then(vpng_json::Json::as_u32),
        Some(48)
    );
    assert_eq!(
        state.get("format").and_then(vpng_json::Json::as_str),
        Some("rgb8")
    );
}

#[test]
fn the_editor_serves_the_picture_and_saves_an_edit() {
    let directory = scratch("edit");
    let picture = make_picture(&directory, 64, 48);
    let editor = Editor::start(&picture);

    // The picture comes back exactly as the file holds it.
    let (status, body) = editor.post(
        "/api/region",
        r#"{"x": 0, "y": 0, "width": 4, "height": 2}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(body.len(), 4 * 2 * 4);
    assert_eq!(&body[0..4], &[0, 0, 0, 255]);
    assert_eq!(&body[4..8], &[1, 0, 0, 255]);

    // The editor fills a rectangle.
    let (status, _) = editor.post("/api/colour", r#"{"r": 7, "g": 6, "b": 5, "a": 255}"#);
    assert_eq!(status, 200);
    let (status, body) = editor.post("/api/fill", r#"{"x": 8, "y": 8, "width": 4, "height": 4}"#);
    assert_eq!(status, 200);
    let report = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(
        report.get("pixels").and_then(vpng_json::Json::as_u64),
        Some(16)
    );

    // The save writes the file.
    let (status, body) = editor.post("/api/save", "{}");
    assert_eq!(status, 200);
    let saved = vpng_json::parse(&String::from_utf8(body).unwrap()).unwrap();
    assert_eq!(
        saved
            .get("tiles_rewritten")
            .and_then(vpng_json::Json::as_u32),
        Some(1)
    );

    // The file on disk is still a valid container, and it holds the edit.
    let bytes = fs::read(&picture).unwrap();
    let document = vpng_decoder::Document::parse(&bytes, &Limits::default()).unwrap();
    let decoded = document.decode_all(&Limits::default()).unwrap();
    assert_eq!(decoded.format(), PixelFormat::rgb8());
    let pixels = decoded.data();
    let index = ((8 * 64 + 8) * 3) as usize;
    assert_eq!(&pixels[index..index + 3], &[7, 6, 5]);
    let outside = 0usize;
    assert_eq!(&pixels[outside..outside + 3], &[0, 0, 0]);
}

#[test]
fn the_editor_refuses_a_request_without_the_token() {
    let directory = scratch("token");
    let picture = make_picture(&directory, 32, 32);
    let editor = Editor::start(&picture);

    // The request carries no token, so the editor refuses it. The client
    // turns a 4xx into an error, so the status is read from the error.
    let outcome = ureq::get(&format!("{}/api/state", editor.base)).call();
    let status = match outcome {
        Ok(response) => response.status().as_u16(),
        Err(ureq::Error::StatusCode(code)) => code,
        Err(other) => panic!("the editor answered oddly: {other}"),
    };
    assert_eq!(status, 400);
}

#[test]
fn the_editor_refuses_a_file_it_cannot_edit() {
    let directory = scratch("refuse");
    // A file that is not a container at all.
    let broken = directory.join("broken.vpng");
    fs::write(&broken, b"not a container at all").unwrap();

    let output = Command::new(VPNG)
        .args(["ui", broken.to_str().unwrap(), "--no-browser"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "the editor opened a broken file");
    assert_eq!(output.status.code(), Some(4), "the corrupt-file exit code");
}

#[test]
fn the_editor_needs_a_file() {
    let output = Command::new(VPNG).args(["ui"]).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        output.status.code(),
        Some(2),
        "the invalid-argument exit code"
    );
}
