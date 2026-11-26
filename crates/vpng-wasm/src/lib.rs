//! `vpng-wasm` — the VPNG decoder as a WebAssembly module.
//!
//! This is the browser's copy of the engine. It is **not** a second
//! implementation: every pixel it produces comes from the same
//! `vpng-decoder` and `vpng-codecs` code the command line runs, through the
//! same shared validation and the same shared tile decoding. That is the
//! whole point of the package — a browser and the command line cannot
//! disagree about what a file means, and the evidence is that the pixels are
//! byte-identical.
//!
//! # The shape of it
//!
//! The module exports a **raw `extern "C"` ABI**. There is no
//! `wasm-bindgen`, no generated glue and no build-time code generation: a
//! browser instantiates the `.wasm` file and calls the functions directly.
//! The JavaScript wrapper in `wasm/vpng.js` turns those calls into an
//! ordinary API.
//!
//! ```text
//! vpng_alloc(len)                  -> pointer
//! vpng_open(data_ptr, len)         -> handle
//! vpng_width(handle)               -> width
//! vpng_decode_region(h, x, y, w, h, out_ptr) -> bytes written
//! vpng_close(handle)               -> 0
//! vpng_dealloc(ptr, len)           -> ()
//! ```
//!
//! The full list is in [`ffi`], and the conventions are:
//!
//! * a **handle** is `i32`; zero is always an error;
//! * a **return value** is `i32`; zero is success and a negative value is
//!   the negative of the error's exit code — **the same codes the command
//!   line uses**;
//! * the **error message** is fetched with `vpng_error_length` and
//!   `vpng_error_copy`;
//! * every **pointer** is checked against the module's memory before it is
//!   used, so a bad pointer from JavaScript is an error and not undefined
//!   behaviour.
//!
//! # Two ways to open a container
//!
//! * `vpng_open` takes the **whole container** in the module's memory. The
//!   module then fetches the payloads it needs out of it. This is the
//!   ordinary case.
//! * `vpng_open_parts` takes the **header and the directory** alone. The
//!   caller — a browser reading the file over HTTP Range requests — then
//!   asks for the payload of the tile it wants with `vpng_tile_offset` and
//!   `vpng_tile_length`, fetches that range, and hands the bytes back with
//!   `vpng_decode_payload`. The payload's checksum is verified against the
//!   directory before the codec sees it, exactly as in the native reader.
//!
//! # The platform
//!
//! The exported functions exist only on a WebAssembly target. On any other
//! target the crate compiles to an empty library, so the workspace still
//! builds and lints on the host, and the CI job builds the `wasm32` target
//! and runs the Node test that proves the pixels are identical.

#![deny(missing_docs)]

mod registry;

#[cfg(target_arch = "wasm32")]
mod ffi;

pub use registry::{Document, Registry};
