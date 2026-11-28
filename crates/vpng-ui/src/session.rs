//! The editing session behind the graphical editor.
//!
//! A session is the **model** the window draws: it owns the file, the
//! editing engine, the current tool and the viewer's plan. It knows nothing
//! about HTTP, JSON or the browser; the protocol layer in
//! [`crate::protocol`] turns its calls into messages and back.
//!
//! # The rules
//!
//! * **The session never holds the whole image.** A display request reads
//!   exactly the rectangle it asks for, through the editing engine's
//!   [`Editor::read_region`], which decodes exactly the tiles that rectangle
//!   intersects. A request that asks for more pixels than the session's cap
//!   allows is refused rather than served.
//! * **Every edit is exact.** A fill writes the colour the user chose into
//!   the file's own pixel format — 8-bit or 16-bit, grey or colour — and the
//!   result is the pixels the user asked for, not an approximation.
//! * **Every edit is undoable.** A drag is one step, and the engine's
//!   tile-level history holds it.
//! * **A save rewrites only what changed.** The commit copies every untouched
//!   tile's payload byte for byte and re-encodes only the dirty ones.
//!
//! # The display is 8-bit; the file is not
//!
//! A browser's canvas takes RGBA8. A file may hold 16-bit samples, a grey
//! channel or no alpha channel at all. The session converts the file's
//! pixels to RGBA8 **for the display only**; an edit is written back in the
//! file's own format, so a 16-bit file stays 16-bit and a grey file stays
//! grey. A file with no alpha channel has nothing for the alpha tool to
//! change, and the tool says so rather than quietly doing nothing.

use std::path::{Path, PathBuf};

use vpng_core::{CanonicalImage, Error, Limits, PixelFormat, Rect, Result};
use vpng_editing::{CommitOptions, CommitReport, Editor, FileSource, TileSource};
use vpng_view::{Direction, PrefetchPolicy, Viewport};

/// The largest number of pixels one display request may return.
///
/// The default is 4,194,304 — a 2048×2048 rectangle. The window asks for the
/// rectangle it can see, and this is the bound on how much of the picture
/// that may be. The bound exists because the display buffer is four bytes per
/// pixel and the source region beside it is one to eight; without a bound, a
/// window showing a whole 16K picture would ask for half a gigabyte.
pub const DEFAULT_MAX_REGION_PIXELS: u64 = 4 * 1024 * 1024;

/// The decoded-tile cache's byte budget in the editor.
pub const DEFAULT_CACHE_BYTES: u64 = 16 * 1024 * 1024;

/// The undo history's byte budget in the editor.
pub const DEFAULT_HISTORY_BYTES: u64 = 16 * 1024 * 1024;

/// The tool the editor is using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// Move the view.
    Pan,
    /// Read the pixel under the pointer.
    Pick,
    /// Fill a rectangle with the current colour.
    Fill,
    /// Set the alpha channel over a rectangle.
    Alpha,
}

impl Tool {
    /// The tool's name, as the protocol spells it.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Pan => "pan",
            Tool::Pick => "pick",
            Tool::Fill => "fill",
            Tool::Alpha => "alpha",
        }
    }

    /// The tool a name names.
    ///
    /// An unknown name is refused: the protocol carries a tool name, and a
    /// name this build does not know is a mistake, not a tool to ignore.
    pub fn from_name(name: &str) -> Result<Self> {
        match name {
            "pan" => Ok(Tool::Pan),
            "pick" => Ok(Tool::Pick),
            "fill" => Ok(Tool::Fill),
            "alpha" => Ok(Tool::Alpha),
            other => Err(Error::invalid(format!(
                "unknown tool `{other}` (expected pan, pick, fill or alpha)"
            ))),
        }
    }
}

/// A colour, in 8 bits per channel.
///
/// The colour is always carried as RGBA8, whatever the file holds. The
/// session converts it to the file's own format when it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    /// The red channel.
    pub red: u8,
    /// The green channel.
    pub green: u8,
    /// The blue channel.
    pub blue: u8,
    /// The alpha channel.
    pub alpha: u8,
}

impl Default for Colour {
    fn default() -> Self {
        Colour {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 255,
        }
    }
}

impl Colour {
    /// The colour as four bytes, red first.
    pub fn to_rgba(self) -> [u8; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }
}

/// One pixel, as the file stores it and as the display shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pixel {
    /// The red channel, scaled to 8 bits for the display.
    pub red: u8,
    /// The green channel, scaled to 8 bits for the display.
    pub green: u8,
    /// The blue channel, scaled to 8 bits for the display.
    pub blue: u8,
    /// The alpha channel, scaled to 8 bits for the display.
    pub alpha: u8,
    /// The file's own samples, in the file's own order.
    ///
    /// A 16-bit file's samples are the 16-bit values; an 8-bit file's are the
    /// 8-bit values. A grey file's are the one grey sample.
    pub raw: Vec<u16>,
}

/// What the viewer engine planned for the last view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlanSummary {
    /// The tiles the view covers.
    pub visible_tiles: usize,
    /// The tiles the prefetch ring covers.
    pub prefetch_tiles: usize,
    /// The prefetch tiles that were actually read into the cache.
    pub prefetched_tiles: usize,
}

/// What an edit did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditReport {
    /// The number of pixels the edit changed.
    pub pixels: u64,
    /// The number of tiles the edit made dirty.
    pub dirty_tiles: usize,
}

/// The session's state, as the window shows it.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// The file's path.
    pub path: PathBuf,
    /// The picture's width.
    pub width: u32,
    /// The picture's height.
    pub height: u32,
    /// The file's pixel format.
    pub format: PixelFormat,
    /// The number of tiles.
    pub tile_count: u32,
    /// The number of levels, including the original.
    pub level_count: usize,
    /// The active tool.
    pub tool: Tool,
    /// The current colour.
    pub colour: Colour,
    /// Whether the document differs from the file.
    pub dirty: bool,
    /// The number of dirty tiles.
    pub dirty_tiles: usize,
    /// The number of edits since the last save.
    pub edits: u64,
    /// Whether there is an edit to undo.
    pub can_undo: bool,
    /// Whether there is an undone edit to redo.
    pub can_redo: bool,
    /// The largest display request the session will serve.
    pub max_region_pixels: u64,
    /// The decoded-tile cache's byte budget.
    pub cache_budget: u64,
    /// The bytes the decoded-tile cache holds.
    pub cache_current_bytes: u64,
    /// The undo history's byte budget.
    pub history_budget: u64,
    /// The bytes the undo history holds.
    pub history_current_bytes: u64,
    /// The last plan the viewer engine made.
    pub plan: PlanSummary,
}

/// The editing session.
#[derive(Debug)]
pub struct Session {
    editor: Editor<FileSource>,
    path: PathBuf,
    tool: Tool,
    colour: Colour,
    policy: PrefetchPolicy,
    max_region_pixels: u64,
    cache_budget: u64,
    history_budget: u64,
    plan: PlanSummary,
}

impl Session {
    /// Open a file for editing.
    ///
    /// The file is validated by the editing engine's own source, which uses
    /// the shared container validation, so a file this session accepts is a
    /// file the decoder accepts. A file that carries a layer table or a GPU
    /// payload table is **refused**, with the reason, because the engine
    /// edits one layer of one grid and cannot rebuild the rest.
    pub fn open(path: &Path, limits: &Limits) -> Result<Self> {
        let source = FileSource::open(path, limits)?;
        let editor = Editor::new(source, limits)?
            .with_cache_bytes(DEFAULT_CACHE_BYTES)?
            .with_history_bytes(DEFAULT_HISTORY_BYTES)?;
        Ok(Session {
            editor,
            path: path.to_path_buf(),
            tool: Tool::Pan,
            colour: Colour::default(),
            policy: PrefetchPolicy::default(),
            max_region_pixels: DEFAULT_MAX_REGION_PIXELS,
            cache_budget: DEFAULT_CACHE_BYTES,
            history_budget: DEFAULT_HISTORY_BYTES,
            plan: PlanSummary::default(),
        })
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The session's state.
    pub fn info(&self) -> SessionInfo {
        let cache = self.editor.cache_stats();
        let history = self.editor.history_stats();
        SessionInfo {
            path: self.path.clone(),
            width: self.editor.width(),
            height: self.editor.height(),
            format: self.editor.format(),
            tile_count: self.editor.tile_count(),
            level_count: self.editor.source().level_count(),
            tool: self.tool,
            colour: self.colour,
            dirty: self.editor.is_dirty(),
            dirty_tiles: self.editor.dirty_tiles().len(),
            edits: self.editor.edit_count(),
            can_undo: history.undo_depth > 0,
            can_redo: history.redo_depth > 0,
            max_region_pixels: self.max_region_pixels,
            cache_budget: self.cache_budget,
            cache_current_bytes: cache.current_bytes,
            history_budget: self.history_budget,
            history_current_bytes: history.current_bytes,
            plan: self.plan,
        }
    }

    /// The active tool.
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Choose the active tool.
    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
    }

    /// The current colour.
    pub fn colour(&self) -> Colour {
        self.colour
    }

    /// Choose the current colour.
    pub fn set_colour(&mut self, colour: Colour) {
        self.colour = colour;
    }

    /// The largest display request the session will serve.
    pub fn max_region_pixels(&self) -> u64 {
        self.max_region_pixels
    }

    /// Set the largest display request the session will serve.
    pub fn set_max_region_pixels(&mut self, pixels: u64) -> Result<()> {
        if pixels == 0 {
            return Err(Error::invalid(
                "the region limit must be at least one pixel",
            ));
        }
        self.max_region_pixels = pixels;
        Ok(())
    }

    /// Read a rectangle of the picture for the display.
    ///
    /// The returned buffer is **RGBA8**, tightly packed, row by row, and it
    /// is exactly `width * height * 4` bytes long. The rectangle is in the
    /// picture's own pixels, so it is the base level — the editor does not
    /// use the preview pyramid, and the report says so.
    ///
    /// The rectangle is refused when it leaves the picture or when it asks
    /// for more pixels than the session's limit allows.
    pub fn region(&mut self, rect: Rect) -> Result<Vec<u8>> {
        self.check_rect(rect)?;
        let image = self.editor.read_region(rect)?;
        let pixels = to_rgba8(&image)?;
        self.plan = self.prefetch(rect)?;
        Ok(pixels)
    }

    /// Read one pixel, exactly as the file stores it.
    pub fn pick(&mut self, x: u32, y: u32) -> Result<Pixel> {
        let dimensions = self.editor.source().dimensions();
        if x >= dimensions.width || y >= dimensions.height {
            return Err(Error::invalid(format!(
                "the pixel ({x}, {y}) is outside the {}x{} picture",
                dimensions.width, dimensions.height
            )));
        }
        let image = self.editor.read_region(Rect::new(x, y, 1, 1)?)?;
        pixel_of(&image)
    }

    /// Fill a rectangle with the current colour.
    ///
    /// The colour is written in the file's own format: a grey file gets the
    /// mean of the colour's three channels, rounded half up; a 16-bit file
    /// gets the 8-bit value scaled by 257, so 255 becomes 65535 exactly. A
    /// file with no alpha channel ignores the colour's alpha.
    pub fn fill(&mut self, rect: Rect) -> Result<EditReport> {
        self.check_rect(rect)?;
        let format = self.editor.format();
        let pixels = fill_pixels(format, rect, self.colour)?;
        self.editor.write_region(rect, &pixels)?;
        Ok(EditReport {
            pixels: rect.area()?,
            dirty_tiles: self.editor.dirty_tiles().len(),
        })
    }

    /// Set the alpha channel over a rectangle.
    ///
    /// A file with no alpha channel is refused: there is nothing for the
    /// tool to change, and saying so is better than doing nothing quietly.
    pub fn set_alpha(&mut self, rect: Rect, value: u8) -> Result<EditReport> {
        self.check_rect(rect)?;
        let format = self.editor.format();
        if !format.color_model.has_alpha() {
            return Err(Error::unsupported(format!(
                "the picture is {} and has no alpha channel",
                format_name(format)
            )));
        }
        let mut image = self.editor.read_region(rect)?;
        {
            let channels = format.channels() as usize;
            let bytes = format.bytes_per_channel() as usize;
            let alpha_index = channels - 1;
            let row = image.row_bytes()? as usize;
            let data = image.data_mut();
            for y in 0..rect.height as usize {
                let start = y * row;
                for x in 0..rect.width as usize {
                    let pixel = start + x * channels * bytes;
                    if bytes == 1 {
                        data[pixel + alpha_index] = value;
                    } else {
                        let wide = value as u16 * 257;
                        data[pixel + alpha_index * 2] = wide as u8;
                        data[pixel + alpha_index * 2 + 1] = (wide >> 8) as u8;
                    }
                }
            }
        }
        self.editor.write_region(rect, image.data())?;
        Ok(EditReport {
            pixels: rect.area()?,
            dirty_tiles: self.editor.dirty_tiles().len(),
        })
    }

    /// Undo the newest edit.
    pub fn undo(&mut self) -> Result<bool> {
        self.editor.undo()
    }

    /// Redo the newest undone edit.
    pub fn redo(&mut self) -> Result<bool> {
        self.editor.redo()
    }

    /// Save the document.
    ///
    /// The commit writes a new file and renames it over the target, so a
    /// failure leaves the original untouched. Every tile the edit did not
    /// touch keeps its payload byte for byte.
    pub fn save(&mut self, target: Option<&Path>) -> Result<CommitReport> {
        let path = target.unwrap_or(self.path.as_path());
        self.editor.commit(path, &CommitOptions::default())
    }

    /// Check that a rectangle is inside the picture and within the limit.
    fn check_rect(&self, rect: Rect) -> Result<()> {
        let dimensions = self.editor.source().dimensions();
        if !rect.is_within(dimensions) {
            return Err(Error::invalid(format!(
                "the rectangle {}x{}+{}+{} is outside the {}x{} picture",
                rect.width, rect.height, rect.x, rect.y, dimensions.width, dimensions.height
            )));
        }
        let area = rect.area()?;
        if area > self.max_region_pixels {
            return Err(Error::resource_limit(format!(
                "the rectangle holds {area} pixels; the editor's limit is {}",
                self.max_region_pixels
            )));
        }
        Ok(())
    }

    /// Plan the view and warm the cache with the prefetch ring.
    ///
    /// The **viewer engine** decides which tiles are on screen and which are
    /// likely to be wanted next; the **editing engine** does the reading, so
    /// there is one decoder in the process. The prefetch reads each tile on
    /// its own, so the transient buffer is one tile rather than the whole
    /// ring.
    fn prefetch(&mut self, rect: Rect) -> Result<PlanSummary> {
        let (visible, ring) = {
            let partition = self.editor.partition();
            let viewport = Viewport::new(partition, rect)?;
            let plan = viewport.plan(&self.policy, Direction::None);
            let mut ring = Vec::with_capacity(plan.prefetch.len());
            for entry in &plan.prefetch {
                if let Some(tile) = partition.tile(entry.tile) {
                    ring.push(tile.rect());
                }
            }
            (plan.visible.len(), ring)
        };

        let mut prefetched = 0;
        for tile in &ring {
            if self.editor.read_region(*tile).is_ok() {
                prefetched += 1;
            }
        }

        Ok(PlanSummary {
            visible_tiles: visible,
            prefetch_tiles: ring.len(),
            prefetched_tiles: prefetched,
        })
    }
}

/// The name of a pixel format, as the report and the window spell it.
pub fn format_name(format: PixelFormat) -> &'static str {
    match (format.color_model, format.bits.bits()) {
        (vpng_core::ColorModel::Gray, 8) => "gray8",
        (vpng_core::ColorModel::Gray, 16) => "gray16",
        (vpng_core::ColorModel::GrayAlpha, 8) => "graya8",
        (vpng_core::ColorModel::GrayAlpha, 16) => "graya16",
        (vpng_core::ColorModel::Rgb, 8) => "rgb8",
        (vpng_core::ColorModel::Rgb, 16) => "rgb16",
        (vpng_core::ColorModel::Rgba, 8) => "rgba8",
        (vpng_core::ColorModel::Rgba, 16) => "rgba16",
        _ => "unsupported",
    }
}

/// Convert an image to RGBA8 for the display.
///
/// The conversion is for the **display only**: a 16-bit sample is scaled
/// down by its top byte, a grey sample is replicated into the three colour
/// channels, and a picture with no alpha channel is shown opaque.
pub fn to_rgba8(image: &CanonicalImage) -> Result<Vec<u8>> {
    let format = image.format();
    let width = image.width() as usize;
    let height = image.height() as usize;
    let channels = format.channels() as usize;
    let bytes = format.bytes_per_channel() as usize;
    let row = image.row_bytes()? as usize;
    let data = image.data();

    let mut out = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let start = y * row;
        for x in 0..width {
            let pixel = &data[start + x * channels * bytes..start + (x + 1) * channels * bytes];
            let sample = |index: usize| -> u16 {
                if bytes == 1 {
                    u16::from(pixel[index])
                } else {
                    u16::from_le_bytes([pixel[index * 2], pixel[index * 2 + 1]])
                }
            };
            let display = |value: u16| -> u8 {
                if bytes == 1 {
                    value as u8
                } else {
                    (value >> 8) as u8
                }
            };
            match format.color_model {
                vpng_core::ColorModel::Gray => {
                    let grey = display(sample(0));
                    out.extend_from_slice(&[grey, grey, grey, 255]);
                }
                vpng_core::ColorModel::GrayAlpha => {
                    let grey = display(sample(0));
                    out.extend_from_slice(&[grey, grey, grey, display(sample(1))]);
                }
                vpng_core::ColorModel::Rgb => {
                    out.extend_from_slice(&[
                        display(sample(0)),
                        display(sample(1)),
                        display(sample(2)),
                        255,
                    ]);
                }
                vpng_core::ColorModel::Rgba => {
                    out.extend_from_slice(&[
                        display(sample(0)),
                        display(sample(1)),
                        display(sample(2)),
                        display(sample(3)),
                    ]);
                }
                vpng_core::ColorModel::Indexed => {
                    return Err(Error::unsupported(
                        "an indexed picture cannot be shown; the format is not implemented",
                    ))
                }
            }
        }
    }
    Ok(out)
}

/// The pixel of a one-pixel image, exactly as the file stores it.
fn pixel_of(image: &CanonicalImage) -> Result<Pixel> {
    let format = image.format();
    let channels = format.channels() as usize;
    let bytes = format.bytes_per_channel() as usize;
    let data = image.data();
    if data.len() < channels * bytes {
        return Err(Error::internal("the pixel buffer is too short"));
    }
    let sample = |index: usize| -> u16 {
        if bytes == 1 {
            u16::from(data[index])
        } else {
            u16::from_le_bytes([data[index * 2], data[index * 2 + 1]])
        }
    };
    let display = |value: u16| -> u8 {
        if bytes == 1 {
            value as u8
        } else {
            (value >> 8) as u8
        }
    };
    let raw: Vec<u16> = (0..channels).map(sample).collect();
    let pixel = match format.color_model {
        vpng_core::ColorModel::Gray => Pixel {
            red: display(sample(0)),
            green: display(sample(0)),
            blue: display(sample(0)),
            alpha: 255,
            raw,
        },
        vpng_core::ColorModel::GrayAlpha => Pixel {
            red: display(sample(0)),
            green: display(sample(0)),
            blue: display(sample(0)),
            alpha: display(sample(1)),
            raw,
        },
        vpng_core::ColorModel::Rgb => Pixel {
            red: display(sample(0)),
            green: display(sample(1)),
            blue: display(sample(2)),
            alpha: 255,
            raw,
        },
        vpng_core::ColorModel::Rgba => Pixel {
            red: display(sample(0)),
            green: display(sample(1)),
            blue: display(sample(2)),
            alpha: display(sample(3)),
            raw,
        },
        vpng_core::ColorModel::Indexed => {
            return Err(Error::unsupported(
                "an indexed picture cannot be shown; the format is not implemented",
            ))
        }
    };
    Ok(pixel)
}

/// The pixels of a filled rectangle, in the file's own format.
fn fill_pixels(format: PixelFormat, rect: Rect, colour: Colour) -> Result<Vec<u8>> {
    let channels = format.channels() as usize;
    let bytes = format.bytes_per_channel() as usize;
    let count = rect.width as usize * rect.height as usize;
    let mut out = Vec::with_capacity(count * channels * bytes);

    // The grey value is the mean of the three colour channels, rounded half
    // up. It is the same rule the report and the window use, so a grey file
    // takes the colour the user sees.
    let grey8 = ((colour.red as u32 + colour.green as u32 + colour.blue as u32 + 1) / 3) as u8;
    let grey16 = grey8 as u16 * 257;

    let sample8 = |value: u8| -> [u8; 2] {
        if bytes == 1 {
            [value, 0]
        } else {
            let wide = value as u16 * 257;
            [wide as u8, (wide >> 8) as u8]
        }
    };

    for _ in 0..count {
        match format.color_model {
            vpng_core::ColorModel::Gray => {
                if bytes == 1 {
                    out.push(grey8);
                } else {
                    out.extend_from_slice(&grey16.to_le_bytes());
                }
            }
            vpng_core::ColorModel::GrayAlpha => {
                if bytes == 1 {
                    out.push(grey8);
                    out.push(colour.alpha);
                } else {
                    out.extend_from_slice(&grey16.to_le_bytes());
                    out.extend_from_slice(&(colour.alpha as u16 * 257).to_le_bytes());
                }
            }
            vpng_core::ColorModel::Rgb => {
                for value in [colour.red, colour.green, colour.blue] {
                    let bytes_out = sample8(value);
                    if bytes == 1 {
                        out.push(bytes_out[0]);
                    } else {
                        out.extend_from_slice(&bytes_out);
                    }
                }
            }
            vpng_core::ColorModel::Rgba => {
                for value in [colour.red, colour.green, colour.blue, colour.alpha] {
                    let bytes_out = sample8(value);
                    if bytes == 1 {
                        out.push(bytes_out[0]);
                    } else {
                        out.extend_from_slice(&bytes_out);
                    }
                }
            }
            vpng_core::ColorModel::Indexed => {
                return Err(Error::unsupported(
                    "an indexed picture cannot be edited; the format is not implemented",
                ))
            }
        }
    }
    Ok(out)
}
