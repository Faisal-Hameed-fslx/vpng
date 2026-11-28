//! The window, embedded in the binary.
//!
//! The front end is four files — a page, a stylesheet, the application
//! script and the geometry module — and they are **compiled into the
//! binary**. The editor is one program the user runs; it does not need a
//! folder of files beside it, and it cannot be pointed at a different front
//! end by accident.
//!
//! The files themselves live in the repository's `ui/` directory, beside the
//! WebAssembly package's `wasm/` directory, because they are read and
//! reviewed as source rather than as build output. The Node test suite in
//! `ui/test/` exercises the geometry module directly.

/// The window's page.
pub const INDEX_HTML: &str = include_str!("../../../ui/index.html");

/// The window's application script.
pub const APP_JS: &str = include_str!("../../../ui/app.js");

/// The window's geometry module.
pub const GEOMETRY_JS: &str = include_str!("../../../ui/geometry.js");

/// The window's stylesheet.
pub const STYLE_CSS: &str = include_str!("../../../ui/style.css");

/// The content type and the text of an asset, by its path.
pub fn asset(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/" | "/index.html" => Some(("text/html; charset=utf-8", INDEX_HTML)),
        "/app.js" => Some(("text/javascript; charset=utf-8", APP_JS)),
        "/geometry.js" => Some(("text/javascript; charset=utf-8", GEOMETRY_JS)),
        "/style.css" => Some(("text/css; charset=utf-8", STYLE_CSS)),
        _ => None,
    }
}
