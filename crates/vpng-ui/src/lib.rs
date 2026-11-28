//! `vpng-ui` — the graphical editor.
//!
//! The editor is a **window** over the two engines the project already has:
//! [`vpng_view`] decides which tiles are on screen and which are wanted next,
//! and [`vpng_editing`] decodes them, changes them and saves them. There is
//! no second decoder in this crate and no second way to write a file: the
//! editor is a view and a set of commands, and the engines do the work.
//!
//! # The window
//!
//! The window is a browser. The editor serves a small page over HTTP on the
//! loopback interface and opens it; the page draws the picture on a canvas
//! and turns the pointer and the keyboard into commands.
//!
//! Why a browser and not a native toolkit:
//!
//! * **The engines are already here.** A native toolkit would add a windowing
//!   stack, an event loop and a drawing library to a workspace that
//!   deliberately has none, and the drawing code would be new code that has
//!   to be tested on its own.
//! * **It can be tested.** The protocol is HTTP and JSON, so the whole
//!   editor — open a file, read a region, fill a rectangle, save — is
//!   exercised by the test suite without a display server. A native window
//!   cannot be tested in the continuous-integration job at all.
//! * **It is not a second implementation.** The page draws bytes the engine
//!   sends and sends commands the engine executes. There is no image codec
//!   in JavaScript and no way for the page to write a file.
//!
//! # What the editor does not do
//!
//! * It does not use the **preview pyramid**: it shows the picture's original
//!   level, and its zoom-out is bounded by a limit on the number of pixels
//!   one request may carry.
//! * It does not edit a file that carries a **layer table** or a **GPU
//!   payload table**: the editing engine refuses those, and the editor
//!   shows the reason.
//!
//! # The parts
//!
//! * [`session`] — the model: the file, the engine, the tool, the colour.
//! * [`protocol`] — the messages between the window and the model.
//! * [`http`] — the small HTTP/1.1 server.
//! * [`server`] — the routes, the trust model and the concurrency.
//! * [`assets`] — the window itself, compiled into the binary.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

pub mod assets;
pub mod http;
pub mod protocol;
pub mod server;
pub mod session;

pub use server::{random_token, Server, TOKEN_HEADER};
pub use session::{
    format_name, to_rgba8, Colour, EditReport, Pixel, PlanSummary, Session, SessionInfo, Tool,
    DEFAULT_CACHE_BYTES, DEFAULT_HISTORY_BYTES, DEFAULT_MAX_REGION_PIXELS,
};
