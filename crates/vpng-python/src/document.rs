//! The document and transaction objects.
//!
//! A `Document` owns a [`FileEditor`]: the editing engine's file-backed
//! session, which reads only the tiles it needs and keeps a bounded cache of
//! decoded tiles. The document and an open transaction share that session, so
//! a transaction can write regions while the document keeps reading.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pyo3::prelude::*;
use pyo3::types::PyDict;
use vpng_core::{Error, Limits, PixelFormat, Rect, Result};
use vpng_editing::{CommitOptions, FileEditor, TileSource};

use crate::error::to_py_result;
use crate::image::{format_name, Region};
use crate::report::CommitResult;

/// The state a document and its transactions share.
struct Shared {
    /// The editing session.
    editor: Mutex<FileEditor>,
    /// The file the session edits.
    path: PathBuf,
}

impl Shared {
    /// Lock the session, turning a poisoned lock into an internal error.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, FileEditor>> {
        self.editor.lock().map_err(|_| {
            Error::internal("the editing session's lock was poisoned by an earlier panic")
        })
    }

    /// The picture's pixel format.
    fn format(&self) -> Result<PixelFormat> {
        let editor = self.lock()?;
        Ok(editor.editor_ref().format())
    }
}

/// An open `.vpng` document.
///
/// The document is a context manager: leaving the `with` block closes it.
///
/// ```python
/// with vpng.open_document("picture.vpng") as doc:
///     region = doc.read_region(x=0, y=0, width=512, height=512)
///     with doc.edit() as tx:
///         tx.write_region(region)
///         tx.commit()
/// ```
#[pyclass(module = "vpng")]
pub struct Document {
    /// The shared editing session.
    shared: Arc<Shared>,
    /// Whether the document has been closed.
    closed: bool,
}

impl Document {
    /// Open a document over a file.
    pub fn open(
        path: &Path,
        limits: &Limits,
        cache_bytes: u64,
        history_bytes: u64,
    ) -> Result<Self> {
        let editor = FileEditor::open(path, limits)?
            .with_cache_bytes(cache_bytes)?
            .with_history_bytes(history_bytes)?;
        Ok(Document {
            shared: Arc::new(Shared {
                editor: Mutex::new(editor),
                path: path.to_path_buf(),
            }),
            closed: false,
        })
    }
}

#[pymethods]
impl Document {
    /// The path of the file the document edits.
    #[getter]
    fn path(&self) -> String {
        self.shared.path.to_string_lossy().into_owned()
    }

    /// The picture's width, in pixels.
    #[getter]
    fn width(&self) -> PyResult<u32> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(editor.editor_ref().width())
    }

    /// The picture's height, in pixels.
    #[getter]
    fn height(&self) -> PyResult<u32> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(editor.editor_ref().height())
    }

    /// The picture's pixel format, as a string such as `rgba8`.
    #[getter]
    fn format(&self) -> PyResult<String> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(format_name(editor.editor_ref().format()))
    }

    /// The number of tiles in the file.
    #[getter]
    fn tile_count(&self) -> PyResult<u32> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(editor.editor_ref().tile_count())
    }

    /// The number of levels: one for a v0.1 file, more for a pyramid.
    #[getter]
    fn level_count(&self) -> PyResult<usize> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(editor.editor_ref().source().level_count())
    }

    /// Whether the document has unsaved edits.
    #[getter]
    fn is_dirty(&self) -> PyResult<bool> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(editor.editor_ref().is_dirty())
    }

    /// The number of edits made since the last commit.
    #[getter]
    fn edit_count(&self) -> PyResult<u64> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(editor.editor_ref().edit_count())
    }

    /// The number of dirty tiles.
    #[getter]
    fn dirty_tiles(&self) -> PyResult<usize> {
        let editor = to_py_result(self.shared.lock())?;
        Ok(editor.editor_ref().dirty_tiles().len())
    }

    /// The header and directory facts about the file.
    ///
    /// This reads the header and the directory only, so it is O(1) in the tile
    /// count — and it cannot detect a corrupted payload. Use `check()` for
    /// that.
    fn info<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let editor = to_py_result(self.shared.lock())?;
        let header = *editor.editor_ref().source().header();
        let info = PyDict::new(py);
        info.set_item("file", self.shared.path.to_string_lossy().into_owned())?;
        info.set_item("file_size", header.declared_file_size)?;
        info.set_item("major", header.major)?;
        info.set_item("minor", header.minor)?;
        info.set_item("width", header.width)?;
        info.set_item("height", header.height)?;
        info.set_item("color_model", header.color_model.name())?;
        info.set_item("bits_per_channel", header.bits.bits())?;
        info.set_item("channels", header.channels())?;
        info.set_item("default_codec", header.default_codec)?;
        info.set_item("tile_count", header.tile_count)?;
        info.set_item("directory_offset", header.directory_offset)?;
        info.set_item("declared_file_size", header.declared_file_size)?;
        info.set_item("feature_flags", header.feature_flags)?;
        info.set_item("payloads_verified", false)?;
        Ok(info)
    }

    /// Read a region of the picture.
    ///
    /// The result holds the region's canonical pixel bytes: rows are tightly
    /// packed, and a 16-bit sample is little-endian. Only the tiles the
    /// region touches are decoded.
    #[pyo3(signature = (x, y, width, height))]
    fn read_region(&self, x: u32, y: u32, width: u32, height: u32) -> PyResult<Region> {
        let rect = to_py_result(Rect::new(x, y, width, height))?;
        let mut editor = to_py_result(self.shared.lock())?;
        let image = to_py_result(editor.editor().read_region(rect))?;
        Ok(Region::from_image(&image, rect))
    }

    /// Write a region of the picture.
    ///
    /// The pixels may be a [`Region`] — in which case its shape is used — or
    /// a `bytes`-like object, in which case `width` and `height` are
    /// required and the bytes are read in **the document's own pixel
    /// format**. The write happens outside a transaction and is not saved
    /// until `commit()` is called.
    #[pyo3(signature = (pixels, x=None, y=None, width=None, height=None))]
    fn write_region(
        &self,
        pixels: &Bound<'_, PyAny>,
        x: Option<u32>,
        y: Option<u32>,
        width: Option<u32>,
        height: Option<u32>,
    ) -> PyResult<()> {
        let format = to_py_result(self.shared.format())?;
        let resolved = resolve_block(pixels, x, y, width, height, Some(format))?;
        let mut editor = to_py_result(self.shared.lock())?;
        to_py_result(editor.editor().write_region(resolved.rect, &resolved.data))?;
        Ok(())
    }

    /// Open a transaction.
    ///
    /// The transaction is a context manager. Leaving the block without
    /// committing **rolls the edits back**, so a failed edit never reaches
    /// the file.
    fn edit(&self) -> PyResult<Transaction> {
        {
            let mut editor = to_py_result(self.shared.lock())?;
            to_py_result(editor.editor().begin())?;
        }
        Ok(Transaction {
            shared: Arc::clone(&self.shared),
            finished: false,
        })
    }

    /// Undo the last edit. Returns `False` when there is nothing to undo.
    fn undo(&self) -> PyResult<bool> {
        let mut editor = to_py_result(self.shared.lock())?;
        to_py_result(editor.editor().undo())
    }

    /// Redo the last undone edit. Returns `False` when there is nothing to
    /// redo.
    fn redo(&self) -> PyResult<bool> {
        let mut editor = to_py_result(self.shared.lock())?;
        to_py_result(editor.editor().redo())
    }

    /// Write the document to a file and replace it atomically.
    ///
    /// Every tile the edit did not touch is copied byte for byte; only the
    /// dirty tiles are re-encoded. A commit that fails changes nothing.
    #[pyo3(signature = (path=None))]
    fn commit(&self, path: Option<PathBuf>) -> PyResult<CommitResult> {
        let target = path.unwrap_or_else(|| self.shared.path.clone());
        let mut editor = to_py_result(self.shared.lock())?;
        let report = to_py_result(editor.editor().commit(&target, &CommitOptions::default()))?;
        Ok(CommitResult::from_report(&report))
    }

    /// Close the document.
    fn close(&mut self) {
        self.closed = true;
    }

    /// Whether the document has been closed.
    #[getter]
    fn is_closed(&self) -> bool {
        self.closed
    }

    /// Enter the context manager.
    fn __enter__(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }

    /// Leave the context manager.
    fn __exit__(
        &mut self,
        _kind: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) {
        self.close();
    }

    /// A readable summary.
    fn __repr__(&self) -> String {
        format!("Document(path={:?})", self.shared.path)
    }
}

/// An open transaction.
///
/// The transaction is a context manager. Leaving the block without
/// committing rolls the edits back.
#[pyclass(module = "vpng")]
pub struct Transaction {
    /// The shared editing session.
    shared: Arc<Shared>,
    /// Whether the transaction has been committed or rolled back.
    finished: bool,
}

#[pymethods]
impl Transaction {
    /// Write a region of the picture.
    ///
    /// The pixels may be a [`Region`] — in which case its shape and position
    /// are used — or a `bytes`-like object, in which case `width` and
    /// `height` are required and the bytes are read in **the document's own
    /// pixel format**.
    #[pyo3(signature = (pixels, x=None, y=None, width=None, height=None))]
    fn write_region(
        &self,
        pixels: &Bound<'_, PyAny>,
        x: Option<u32>,
        y: Option<u32>,
        width: Option<u32>,
        height: Option<u32>,
    ) -> PyResult<()> {
        let format = to_py_result(self.shared.format())?;
        let resolved = resolve_block(pixels, x, y, width, height, Some(format))?;
        let mut editor = to_py_result(self.shared.lock())?;
        to_py_result(editor.editor().write_region(resolved.rect, &resolved.data))?;
        Ok(())
    }

    /// Write the document to a file and replace it atomically.
    #[pyo3(signature = (path=None))]
    fn commit(&mut self, path: Option<PathBuf>) -> PyResult<CommitResult> {
        if self.finished {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "the transaction has already been committed or rolled back",
            ));
        }
        let target = path.unwrap_or_else(|| self.shared.path.clone());
        let mut editor = to_py_result(self.shared.lock())?;
        let report = to_py_result(editor.editor().commit(&target, &CommitOptions::default()))?;
        self.finished = true;
        Ok(CommitResult::from_report(&report))
    }

    /// Throw the transaction's edits away.
    fn rollback(&mut self) -> PyResult<()> {
        if self.finished {
            return Err(pyo3::exceptions::PyRuntimeError::new_err(
                "the transaction has already been committed or rolled back",
            ));
        }
        let mut editor = to_py_result(self.shared.lock())?;
        to_py_result(editor.editor().abort())?;
        self.finished = true;
        Ok(())
    }

    /// Enter the context manager.
    fn __enter__(slf: Bound<'_, Self>) -> Bound<'_, Self> {
        slf
    }

    /// Leave the context manager, rolling back an uncommitted transaction.
    fn __exit__(
        &mut self,
        _kind: Option<&Bound<'_, PyAny>>,
        _value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
    ) {
        if !self.finished {
            if let Ok(mut editor) = self.shared.lock() {
                let _ = editor.editor().abort();
            }
            self.finished = true;
        }
    }

    /// A readable summary.
    fn __repr__(&self) -> String {
        format!("Transaction(path={:?})", self.shared.path)
    }
}

/// The pixels and the rectangle they go into.
struct Resolved {
    /// The canonical pixel bytes.
    data: Vec<u8>,
    /// The rectangle they cover.
    rect: Rect,
}

/// Work out what a Python object means as a block of pixels, and where it
/// goes.
fn resolve_block(
    pixels: &Bound<'_, PyAny>,
    x: Option<u32>,
    y: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    fallback_format: Option<PixelFormat>,
) -> PyResult<Resolved> {
    // A region carries its own shape and position.
    if let Ok(region) = pixels.extract::<PyRef<'_, Region>>() {
        let rect = Rect::new(
            x.unwrap_or_else(|| region.rect_x()),
            y.unwrap_or_else(|| region.rect_y()),
            width.unwrap_or_else(|| region.rect_width()),
            height.unwrap_or_else(|| region.rect_height()),
        )
        .map_err(|error| crate::error::to_py_err(&error))?;
        return Ok(Resolved {
            data: region.bytes().to_vec(),
            rect,
        });
    }

    // Raw bytes need a shape.
    let data: Vec<u8> = pixels.extract::<Vec<u8>>().map_err(|_| {
        pyo3::exceptions::PyTypeError::new_err(
            "the pixels must be a vpng.Region or a bytes-like object",
        )
    })?;
    let (Some(width), Some(height)) = (width, height) else {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "raw pixel bytes need an explicit width and height",
        ));
    };
    let format = fallback_format.unwrap_or(PixelFormat::rgba8());
    let expected = format
        .frame_bytes(width, height)
        .map_err(|error| crate::error::to_py_err(&error))?;
    if data.len() as u64 != expected {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "the pixel block is {} bytes but {expected} were expected for {width}x{height} {}",
            data.len(),
            format_name(format)
        )));
    }
    let rect = Rect::new(x.unwrap_or(0), y.unwrap_or(0), width, height)
        .map_err(|error| crate::error::to_py_err(&error))?;
    Ok(Resolved { data, rect })
}
