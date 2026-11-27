//! The reports the Python API returns.

use pyo3::prelude::*;
use pyo3::types::PyTuple;
use vpng_editing::CommitReport;
use vpng_encoder::stream::StreamEncodeReport;

/// What an encode did.
#[pyclass(module = "vpng")]
#[derive(Clone)]
pub struct EncodeResult {
    /// The size of the written file, in bytes.
    file_size: u64,
    /// The number of tiles in the file.
    tile_count: u32,
    /// The number of levels: one for a v0.1 file, more for a pyramid.
    level_count: u32,
    /// The minor version the file's header carries.
    format_minor: u16,
    /// The number of layers in the file: one for every file written before
    /// format v0.4.
    layer_count: u32,
    /// The codec of every tile, in tile order.
    codec_ids: Vec<u8>,
    /// The uncompressed pixel bytes over the file bytes.
    compression_ratio: f64,
    /// The number of payload bytes in the file.
    payload_bytes: u64,
    /// How long the encode took, in milliseconds.
    encode_ms: u64,
    /// How many workers the encode used.
    workers: usize,
}

#[pymethods]
impl EncodeResult {
    /// The size of the written file, in bytes.
    #[getter]
    fn file_size(&self) -> u64 {
        self.file_size
    }

    /// The number of tiles in the file.
    #[getter]
    fn tile_count(&self) -> u32 {
        self.tile_count
    }

    /// The number of levels: one for a v0.1 file, more for a pyramid.
    #[getter]
    fn level_count(&self) -> u32 {
        self.level_count
    }

    /// The minor version the file's header carries: 1, 2, 3 or 4.
    #[getter]
    fn format_minor(&self) -> u16 {
        self.format_minor
    }

    /// The number of layers in the file.
    ///
    /// A file written before format v0.4 has exactly one layer: the base
    /// layer, which is the picture itself.
    #[getter]
    fn layer_count(&self) -> u32 {
        self.layer_count
    }

    /// The codec of every tile, in tile order.
    #[getter]
    fn codec_ids<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.codec_ids.iter().copied())
    }

    /// The uncompressed pixel bytes over the file bytes.
    ///
    /// The numerator is the **original** picture's pixels and the denominator
    /// is the whole file, so a pyramid can never flatter the number.
    #[getter]
    fn compression_ratio(&self) -> f64 {
        self.compression_ratio
    }

    /// The number of payload bytes in the file.
    #[getter]
    fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }

    /// How long the encode took, in milliseconds.
    #[getter]
    fn encode_ms(&self) -> u64 {
        self.encode_ms
    }

    /// How many workers the encode used.
    #[getter]
    fn workers(&self) -> usize {
        self.workers
    }

    /// A readable summary.
    fn __repr__(&self) -> String {
        format!(
            "EncodeResult(file_size={}, tile_count={}, format_minor={}, ratio={:.4})",
            self.file_size, self.tile_count, self.format_minor, self.compression_ratio
        )
    }
}

impl EncodeResult {
    /// Build the report from the encoder's own report.
    pub fn from_stream(report: &StreamEncodeReport) -> Self {
        EncodeResult {
            file_size: report.file_size,
            tile_count: report.tile_count,
            level_count: report.level_count,
            format_minor: 0,
            layer_count: report.layer_count,
            codec_ids: report.codec_ids.clone(),
            compression_ratio: report.compression_ratio(),
            payload_bytes: report.payload_bytes,
            encode_ms: report.encode_ms,
            workers: report.workers,
        }
    }

    /// The minor version the written file carries.
    ///
    /// The streaming writer decides the version from what the file contains,
    /// so the header is read back rather than guessed.
    pub fn with_minor(mut self, minor: u16) -> Self {
        self.format_minor = minor;
        self
    }
}

/// What a decode produced.
#[pyclass(module = "vpng")]
#[derive(Clone)]
pub struct DecodeResult {
    /// The width of the decoded picture, in pixels.
    width: u32,
    /// The height of the decoded picture, in pixels.
    height: u32,
    /// The picture's pixel format, as a string such as `rgba8`.
    format: String,
    /// The size of the file that was read, in bytes.
    file_size: u64,
    /// The level that was decoded: zero is the original.
    level: u32,
    /// The number of bytes written to the output, when there was one.
    written_bytes: u64,
}

#[pymethods]
impl DecodeResult {
    /// The width of the decoded picture, in pixels.
    #[getter]
    fn width(&self) -> u32 {
        self.width
    }

    /// The height of the decoded picture, in pixels.
    #[getter]
    fn height(&self) -> u32 {
        self.height
    }

    /// The picture's pixel format, as a string such as `rgba8`.
    #[getter]
    fn format(&self) -> String {
        self.format.clone()
    }

    /// The size of the file that was read, in bytes.
    #[getter]
    fn file_size(&self) -> u64 {
        self.file_size
    }

    /// The level that was decoded: zero is the original.
    #[getter]
    fn level(&self) -> u32 {
        self.level
    }

    /// The number of bytes written to the output, when there was one.
    #[getter]
    fn written_bytes(&self) -> u64 {
        self.written_bytes
    }

    /// A readable summary.
    fn __repr__(&self) -> String {
        format!(
            "DecodeResult(width={}, height={}, format={}, level={})",
            self.width, self.height, self.format, self.level
        )
    }
}

impl DecodeResult {
    /// Build the report from a decoded picture.
    pub fn new(
        width: u32,
        height: u32,
        format: String,
        file_size: u64,
        level: u32,
        written_bytes: u64,
    ) -> Self {
        DecodeResult {
            width,
            height,
            format,
            file_size,
            level,
            written_bytes,
        }
    }
}

/// What an integrity check did.
#[pyclass(module = "vpng")]
#[derive(Clone)]
pub struct CheckResult {
    /// The number of tiles whose payload checksum was verified.
    tiles_checked: u32,
    /// The number of payload bytes that were verified.
    payload_bytes: u64,
}

#[pymethods]
impl CheckResult {
    /// The number of tiles whose payload checksum was verified.
    #[getter]
    fn tiles_checked(&self) -> u32 {
        self.tiles_checked
    }

    /// The number of payload bytes that were verified.
    #[getter]
    fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }

    /// Always true: a failed check raises instead of returning.
    #[getter]
    fn ok(&self) -> bool {
        true
    }

    /// A readable summary.
    fn __repr__(&self) -> String {
        format!(
            "CheckResult(tiles_checked={}, payload_bytes={})",
            self.tiles_checked, self.payload_bytes
        )
    }
}

impl CheckResult {
    /// Build the report from the decoder's own report.
    pub fn new(tiles_checked: u32, payload_bytes: u64) -> Self {
        CheckResult {
            tiles_checked,
            payload_bytes,
        }
    }
}

/// What a save did.
#[pyclass(module = "vpng")]
#[derive(Clone)]
pub struct CommitResult {
    /// The path that was written.
    path: String,
    /// The size of the new file, in bytes.
    file_size: u64,
    /// The number of tiles that were re-encoded.
    tiles_rewritten: u32,
    /// The number of tiles whose payload was copied verbatim.
    tiles_copied: u32,
    /// The bytes copied verbatim from the original file.
    copied_payload_bytes: u64,
    /// The bytes of newly encoded payloads.
    rewritten_payload_bytes: u64,
    /// How many preview levels the commit dropped.
    preview_levels_dropped: u32,
    /// Whether the session's baseline moved to the committed file.
    baseline_advanced: bool,
    /// How long the commit took, in milliseconds.
    elapsed_ms: u64,
}

#[pymethods]
impl CommitResult {
    /// The path that was written.
    #[getter]
    fn path(&self) -> String {
        self.path.clone()
    }

    /// The size of the new file, in bytes.
    #[getter]
    fn file_size(&self) -> u64 {
        self.file_size
    }

    /// The number of tiles that were re-encoded.
    #[getter]
    fn tiles_rewritten(&self) -> u32 {
        self.tiles_rewritten
    }

    /// The number of tiles whose payload was copied verbatim.
    #[getter]
    fn tiles_copied(&self) -> u32 {
        self.tiles_copied
    }

    /// The bytes copied verbatim from the original file.
    #[getter]
    fn copied_payload_bytes(&self) -> u64 {
        self.copied_payload_bytes
    }

    /// The bytes of newly encoded payloads.
    #[getter]
    fn rewritten_payload_bytes(&self) -> u64 {
        self.rewritten_payload_bytes
    }

    /// How many preview levels the commit dropped.
    ///
    /// A commit drops the previews: they describe the picture **before** the
    /// edit, so keeping them would leave the file saying two things at once.
    #[getter]
    fn preview_levels_dropped(&self) -> u32 {
        self.preview_levels_dropped
    }

    /// Whether the session's baseline moved to the committed file.
    #[getter]
    fn baseline_advanced(&self) -> bool {
        self.baseline_advanced
    }

    /// How long the commit took, in milliseconds.
    #[getter]
    fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    /// A readable summary.
    fn __repr__(&self) -> String {
        format!(
            "CommitResult(path={:?}, file_size={}, rewritten={}, copied={})",
            self.path, self.file_size, self.tiles_rewritten, self.tiles_copied
        )
    }
}

impl CommitResult {
    /// Build the report from the editing engine's own report.
    pub fn from_report(report: &CommitReport) -> Self {
        CommitResult {
            path: report.path.to_string_lossy().into_owned(),
            file_size: report.file_size,
            tiles_rewritten: report.tiles_rewritten,
            tiles_copied: report.tiles_copied,
            copied_payload_bytes: report.copied_payload_bytes,
            rewritten_payload_bytes: report.rewritten_payload_bytes,
            preview_levels_dropped: report.preview_levels_dropped,
            baseline_advanced: report.baseline_advanced,
            elapsed_ms: report.elapsed.as_millis() as u64,
        }
    }
}
