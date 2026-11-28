//! The CLI's layer commands.
//!
//! A **layer** is one grid of tiles that can be decoded on its own. A v0.4
//! file holds several of them: the base layer is the image, and the others
//! sit on top of it, each with a name, a rectangle on the canvas, an opacity
//! and — optionally — a mask.
//!
//! Four commands cover them:
//!
//! * `vpng layers` — what the file holds;
//! * `vpng decode-layer` — one layer's own pixels;
//! * `vpng composite` — the layers blended together;
//! * `vpng encode-layers` — build a layered file from a manifest.

use std::path::{Path, PathBuf};

use vpng_container::{canonical_digest, to_hex};
use vpng_core::{Error, Limits, Rect, Result};
use vpng_decoder::Document;
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::stream::encode_layers_to_file;
use vpng_encoder::{EncodeOptions, LayerInput};

use crate::args::Args;
use crate::json::{self, Json};
use crate::{escape, limits, write_output_image};

/// List the layers of a file.
pub fn cmd_layers(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let limits = limits();

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;

    if args.has("json") {
        let mut parts = Vec::new();
        for layer in document.layers() {
            parts.push(format!(
                "{{\"index\":{},\"name\":\"{}\",\"kind\":\"{}\",\"is_mask\":{},\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"opacity\":{},\"mask\":{},\"tile_count\":{},\"uncompressed_bytes\":{},\"compressed_bytes\":{},\"ratio\":{:.6}}}",
                layer.index(),
                escape(layer.name()),
                if layer.is_mask() { "mask" } else { "pixels" },
                layer.is_mask(),
                layer.rect().x,
                layer.rect().y,
                layer.rect().width,
                layer.rect().height,
                layer.opacity(),
                match layer.mask() {
                    Some(index) => index.to_string(),
                    None => "null".to_string(),
                },
                layer.base_level().tile_count(),
                layer.uncompressed_bytes(),
                layer.compressed_bytes(),
                layer.ratio(),
            ));
        }
        println!(
            "{{\"file\":\"{}\",\"layer_count\":{},\"layers\":[{}]}}",
            escape(&input),
            document.layer_count(),
            parts.join(",")
        );
        return Ok(());
    }

    println!("file              {input}");
    println!(
        "format version    {}.{}",
        document.header().major,
        document.header().minor
    );
    println!("layers            {}", document.layer_count());
    for layer in document.layers() {
        let rect = layer.rect();
        let mask = match layer.mask() {
            Some(index) => index.to_string(),
            None => "-".to_string(),
        };
        println!(
            "  layer {:<2} {:<12} {:<6} {}+{} {}x{}  opacity {:<3} mask {:<4} {:>5} tiles  {} B -> {} B  ratio {:.3}x",
            layer.index(),
            layer.name(),
            if layer.is_mask() { "mask" } else { "pixels" },
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            layer.opacity(),
            mask,
            layer.base_level().tile_count(),
            layer.uncompressed_bytes(),
            layer.compressed_bytes(),
            layer.ratio(),
        );
    }
    Ok(())
}

/// Decode one layer's own pixels.
pub fn cmd_decode_layer(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let output = args.require_positional(1, "output path")?.to_string();
    let limits = limits();

    let position = match args.get_u32("layer")? {
        Some(layer) => layer as usize,
        None => 0,
    };

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    if position >= document.layer_count() {
        return Err(Error::invalid(format!(
            "the file has {} layer(s); there is no layer {position}",
            document.layer_count()
        )));
    }

    let image = document.decode_layer(position, &limits)?;
    write_output_image(args, &output, &image)?;

    if args.has("json") {
        println!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"layer\":{},\"width\":{},\"height\":{},\"format\":\"{}\",\"canonical_sha256\":\"{}\"}}",
            escape(&input),
            escape(&output),
            position,
            image.width(),
            image.height(),
            image.format(),
            to_hex(&canonical_digest(&image)),
        );
    } else {
        let layer = document
            .layer(position)
            .expect("the layer was checked above");
        println!(
            "decoded layer {position} (`{}`) -> {output}  ({}x{} {})",
            layer.name(),
            image.width(),
            image.height(),
            image.format()
        );
    }
    Ok(())
}

/// Composite every layer into one image.
pub fn cmd_composite(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let output = args.require_positional(1, "output path")?.to_string();
    let limits = limits();

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    let image = document.composite(&limits)?;
    write_output_image(args, &output, &image)?;

    if args.has("json") {
        println!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"layers\":{},\"width\":{},\"height\":{},\"format\":\"{}\",\"canonical_sha256\":\"{}\"}}",
            escape(&input),
            escape(&output),
            document.layer_count(),
            image.width(),
            image.height(),
            image.format(),
            to_hex(&canonical_digest(&image)),
        );
    } else {
        println!(
            "composited {} layer(s) -> {output}  ({}x{} {})",
            document.layer_count(),
            image.width(),
            image.height(),
            image.format()
        );
    }
    Ok(())
}

/// Build a layered file from a manifest.
///
/// The manifest is a JSON document:
///
/// ```json
/// {
///   "layers": [
///     {"name": "base", "path": "base.png"},
///     {"name": "ink", "path": "ink.png", "x": 20, "y": 20,
///      "width": 100, "height": 60, "mask": 2},
///     {"name": "m", "kind": "mask", "path": "mask.png",
///      "x": 20, "y": 20, "width": 120, "height": 80}
///   ]
/// }
/// ```
///
/// Every layer needs a `path`. The rest have defaults: the name is empty,
/// the kind is `pixels`, the rectangle starts at the origin, the opacity is
/// 255 and there is no mask. A path is resolved against the manifest's own
/// directory, so a manifest and its images can travel together.
pub fn cmd_encode_layers(args: &Args) -> Result<()> {
    let output = args.require_positional(0, "output path")?.to_string();
    let manifest_path = args.require("manifest")?.to_string();
    let limits = limits();

    let manifest_text = std::fs::read_to_string(&manifest_path)?;
    let manifest = json::parse(&manifest_text)?;
    if manifest.as_object().is_none() {
        return Err(Error::invalid(format!(
            "the manifest must be a JSON object, not {}",
            manifest.kind()
        )));
    }
    let directory = std::path::Path::new(&manifest_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let entries = manifest
        .get("layers")
        .and_then(Json::as_array)
        .ok_or_else(|| Error::invalid("the manifest needs a `layers` array at its top level"))?;
    if entries.is_empty() {
        return Err(Error::invalid("the manifest's `layers` array is empty"));
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.as_object().is_none() {
            return Err(Error::invalid(format!(
                "layer {index} must be a JSON object, not {}",
                entry.kind()
            )));
        }
    }

    // The images are read first, because the layer's rectangle must be the
    // size of the pixels it holds.
    let mut images = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let path = entry
            .get("path")
            .and_then(Json::as_str)
            .ok_or_else(|| Error::invalid(format!("layer {index} needs a `path`")))?;
        let resolved = resolve(&directory, path);
        let image = read_layer_image(entry, index, &resolved, &limits)?;
        images.push((entry.clone(), image));
    }

    let mut options = EncodeOptions::default().with_limits(limits);
    if let Some(edge) = args.get_u32("tile-size")? {
        if edge == 0 {
            return Err(Error::invalid("--tile-size must be non-zero"));
        }
        options = options.with_tile_size(edge, edge);
    }
    if let Some(codec) = args.get("codec") {
        options = options.with_codec(crate::parse_codec(codec)?);
    }
    if let Some(set) = args.get("codec-set") {
        options = options.with_codec_set(crate::parse_codec_set(set)?);
    }
    if args.has("gpu") {
        options = options.with_gpu();
    }

    let mut layers: Vec<LayerInput<'_>> = Vec::with_capacity(images.len());
    for (index, (entry, image)) in images.iter().enumerate() {
        let name = entry
            .get("name")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string();
        let kind = entry.get("kind").and_then(Json::as_str).unwrap_or("pixels");
        let x = entry.get("x").and_then(Json::as_u64).unwrap_or(0) as u32;
        let y = entry.get("y").and_then(Json::as_u64).unwrap_or(0) as u32;
        let width = entry
            .get("width")
            .and_then(Json::as_u64)
            .unwrap_or(image.width() as u64) as u32;
        let height = entry
            .get("height")
            .and_then(Json::as_u64)
            .unwrap_or(image.height() as u64) as u32;
        let opacity = entry
            .get("opacity")
            .and_then(Json::as_u64)
            .unwrap_or(255)
            .min(255) as u8;
        let rect = Rect::new(x, y, width, height)?;

        let mut layer = match kind {
            "pixels" | "image" => LayerInput::pixels(name, rect, image),
            "mask" => LayerInput::mask(name, rect, image),
            other => {
                return Err(Error::invalid(format!(
                    "layer {index} has the kind `{other}`; it must be `pixels` or `mask`"
                )))
            }
        };
        layer.opacity = opacity;
        if let Some(mask) = entry.get("mask") {
            if !mask.is_null() {
                let index = mask.as_u64().ok_or_else(|| {
                    Error::invalid(format!("layer {index}'s `mask` must be a layer index"))
                })?;
                layer = layer.with_mask(index as usize);
            }
        }
        layers.push(layer);
    }

    let workers = match args.get_u32("workers")? {
        Some(0) => return Err(Error::invalid("--workers must be non-zero")),
        Some(count) => count as usize,
        None => std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1),
    };
    let parallel = ParallelOptions::default().with_workers(workers);

    let report =
        encode_layers_to_file(&layers, &options, &parallel, std::path::Path::new(&output))?;

    if args.has("json") {
        println!(
            "{{\"output\":\"{}\",\"manifest\":\"{}\",\"layers\":{},\"tiles\":{},\"file_size\":{},\"workers\":{},\"gpu\":{},\"gpu_payload_bytes\":{},\"gpu_table_bytes\":{}}}",
            escape(&output),
            escape(&manifest_path),
            report.layer_count,
            report.tile_count,
            report.file_size,
            workers,
            options.gpu,
            report.gpu_payload_bytes,
            report.gpu_table_bytes,
        );
    } else {
        println!("encoded {manifest_path} -> {output}");
        println!(
            "  {} layer(s), {} tiles, {} bytes, {workers} workers",
            report.layer_count, report.tile_count, report.file_size,
        );
        if options.gpu {
            println!(
                "  GPU payloads: {} B, GPU table: {} B (LOSSY)",
                report.gpu_payload_bytes, report.gpu_table_bytes,
            );
        }
        for (index, layer) in report.layer_uncompressed_bytes.iter().enumerate() {
            let compressed = report
                .layer_compressed_bytes
                .get(index)
                .copied()
                .unwrap_or(0);
            println!(
                "  layer {index}: {} B -> {} B  ratio {:.3}x",
                layer,
                compressed,
                if compressed == 0 {
                    1.0
                } else {
                    *layer as f64 / compressed as f64
                }
            );
        }
    }
    Ok(())
}

/// Resolve a path in the manifest against the manifest's own directory.
fn resolve(directory: &Path, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        directory.join(candidate)
    }
}

/// Read one layer's image.
///
/// A PNG carries its own size and format. A **raw** file carries neither, so
/// the manifest must say how wide and tall it is and what pixel format it
/// holds.
fn read_layer_image(
    entry: &Json,
    index: usize,
    path: &Path,
    limits: &Limits,
) -> Result<vpng_core::CanonicalImage> {
    let text = path.to_string_lossy().to_string();
    let kind = match vpng_image::kind_from_path(&text) {
        Some(kind) => kind,
        None => {
            return Err(Error::invalid(format!(
                "layer {index}'s path `{text}` has no known extension (use .png or .raw)"
            )))
        }
    };

    match kind {
        vpng_image::FileKind::Png => vpng_image::read_png(&text, limits),
        vpng_image::FileKind::Raw => {
            let width = entry
                .get("width")
                .and_then(Json::as_u64)
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "layer {index} reads a raw file, so it needs a `width`"
                    ))
                })? as u32;
            let height = entry
                .get("height")
                .and_then(Json::as_u64)
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "layer {index} reads a raw file, so it needs a `height`"
                    ))
                })? as u32;
            let format = entry
                .get("format")
                .and_then(Json::as_str)
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "layer {index} reads a raw file, so it needs a `format` such as rgba8"
                    ))
                })?;
            vpng_image::read_raw(
                &text,
                width,
                height,
                vpng_image::parse_format(format)?,
                limits,
            )
        }
        vpng_image::FileKind::Vpng => Err(Error::unsupported(
            "a layer's pixels may not come from a .vpng file; the manifest must name the original images",
        )),
    }
}
