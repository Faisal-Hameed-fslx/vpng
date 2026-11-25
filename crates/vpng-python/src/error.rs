//! The Python exception hierarchy.
//!
//! The exception **classes** live in Python (`python/vpng/__init__.py`), not
//! here, because Python allows a class to have more than one base and this
//! library wants both: `vpng.InvalidArgumentError` is a `vpng.VpngError`
//! *and* a `ValueError`, so either `except` clause catches it.
//!
//! The extension therefore looks the classes up once, when the Python wrapper
//! imports it, and raises the right one. If the extension is imported on its
//! own — `import vpng._vpng` — the lookup finds nothing and every error
//! becomes a plain `vpng.VpngError` with the kind in its message.

use std::sync::OnceLock;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use vpng_core::Error;
create_exception!(
    vpng,
    VpngError,
    PyException,
    "The base class of every error VPNG raises.\n\n\
     The Python wrapper replaces this with a richer hierarchy in which each \
     error is also a builtin Python exception; see `python/vpng/__init__.py`."
);

/// The exception classes the Python wrapper registered, keyed by the stable
/// error label.
static EXCEPTIONS: OnceLock<Py<PyDict>> = OnceLock::new();

/// Register the Python exception classes.
///
/// This is called once, by `python/vpng/__init__.py`, with a mapping from the
/// stable error label (`"invalid-argument"`, `"corrupt-file"`, ...) to the
/// exception class that should carry it.
pub fn register_exception_classes(mapping: &Bound<'_, PyDict>) -> PyResult<()> {
    EXCEPTIONS
        .set(mapping.clone().unbind())
        .map_err(|_| PyRuntimeError::new_err("the VPNG exception classes are already registered"))
}

/// The message an error carries, without its kind.
fn message_of(error: &Error) -> String {
    match error {
        Error::InvalidArgument(message)
        | Error::UnsupportedFeature(message)
        | Error::CorruptFile(message)
        | Error::ResourceLimit(message)
        | Error::VerificationFailed(message)
        | Error::Internal(message) => message.clone(),
        Error::SizeLimitExceeded { limit, smallest } => format!(
            "no lossless representation fits inside {limit} bytes \
             (the smallest the enabled codecs can produce is {smallest} bytes)"
        ),
        Error::Io(source) => source.to_string(),
    }
}

/// Turn a VPNG error into the Python exception that matches it.
pub fn to_py_err(error: &Error) -> PyErr {
    let kind = error.kind();
    let message = message_of(error);

    Python::attach(|py| {
        if let Some(mapping) = EXCEPTIONS.get() {
            let mapping = mapping.bind(py);
            if let Ok(Some(class)) = mapping.get_item(kind.label()) {
                if let Ok(instance) = class.call1((message.as_str(),)) {
                    let _ = instance.setattr("kind", kind.label());
                    let _ = instance.setattr("message", message.as_str());
                    return PyErr::from_value(instance);
                }
            }
        }
        // The extension was used without its Python wrapper: fall back to the
        // single Rust exception, with the kind in the message.
        VpngError::new_err(format!("{}: {message}", kind.label()))
    })
}

/// The result of a fallible VPNG operation, as a Python exception.
pub fn to_py_result<T>(result: vpng_core::Result<T>) -> PyResult<T> {
    result.map_err(|error| to_py_err(&error))
}

/// The result of a fallible I/O operation, as a Python exception.
pub fn io_result<T>(result: std::io::Result<T>) -> PyResult<T> {
    result.map_err(|error| to_py_err(&Error::Io(error)))
}
