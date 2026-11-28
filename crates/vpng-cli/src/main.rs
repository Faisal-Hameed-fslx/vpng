//! `vpng` — the VPNG command line tool.
//!
//! Exit codes are stable and documented in `docs/VPNG_SPEC_0_1.md`:
//! 0 success, 1 generic, 2 invalid arguments, 3 unsupported feature,
//! 4 corrupt file, 5 resource limit, 6 size limit, 7 verification failure,
//! 8 I/O failure.

#![forbid(unsafe_code)]

mod args;
mod gpu;
mod layers;
mod size;
mod ui;

use std::process::ExitCode;

use args::Args;
use vpng_container::{canonical_digest, to_hex, Header};
use vpng_core::{Error, Limits, Rect, Result};
use vpng_decoder::Document;
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::EncodeOptions;
use vpng_image as convert;
use vpng_image::FileKind;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vpng: error: {err}");
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

fn run(argv: &[String]) -> Result<()> {
    // A bare `--help`/`--version` with no command is allowed.
    if argv.is_empty() {
        print_help();
        return Ok(());
    }
    if argv[0] == "--version" || argv[0] == "-V" {
        println!("vpng {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let args = Args::parse(argv)?;

    if args.has("help") {
        print_help();
        return Ok(());
    }

    match args.command.as_str() {
        "encode" => cmd_encode(&args),
        "encode-layers" => layers::cmd_encode_layers(&args),
        "decode" => cmd_decode(&args),
        "decode-layer" => layers::cmd_decode_layer(&args),
        "composite" => layers::cmd_composite(&args),
        "layers" => layers::cmd_layers(&args),
        "gpu-info" => gpu::cmd_gpu_info(&args),
        "gpu-decode" => gpu::cmd_gpu_decode(&args),
        "ui" => ui::cmd_ui(&args),
        "info" => cmd_info(&args),
        "check" => cmd_check(&args),
        "verify" => cmd_verify(&args),
        "decode-region" => cmd_decode_region(&args),
        "extract-tile" => cmd_extract_tile(&args),
        "http-info" => cmd_http_info(&args),
        "http-decode-region" => cmd_http_decode_region(&args),
        "help" => {
            print_help();
            Ok(())
        }
        "version" => {
            println!("vpng {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(Error::invalid(format!(
            "unknown command `{other}`; run `vpng help`"
        ))),
    }
}

fn limits() -> Limits {
    Limits::default()
}

/// Read an image in PNG, raw, or VPNG form, honoring `--input-format`.
fn read_input_image(args: &Args, path: &str, limits: &Limits) -> Result<vpng_core::CanonicalImage> {
    let kind = match args.get("input-format") {
        Some(text) => convert::parse_kind(text)?,
        None => convert::kind_from_path(path).ok_or_else(|| {
            Error::invalid(format!(
                "cannot infer the format of `{path}`; pass --input-format png|raw|vpng"
            ))
        })?,
    };

    match kind {
        FileKind::Png => convert::read_png(path, limits),
        FileKind::Raw => {
            let width = args.require_u32("width")?;
            let height = args.require_u32("height")?;
            let format = convert::parse_format(args.require("format")?)?;
            convert::read_raw(path, width, height, format, limits)
        }
        FileKind::Vpng => {
            let bytes = std::fs::read(path)?;
            let document = Document::parse(&bytes, limits)?;
            document.decode_all(limits)
        }
    }
}

/// Write an image, defaulting to PNG unless the target is `.raw`.
fn write_output_image(args: &Args, path: &str, image: &vpng_core::CanonicalImage) -> Result<()> {
    let kind = match args.get("format") {
        Some(text) => match text.to_ascii_lowercase().as_str() {
            "raw" => FileKind::Raw,
            "png" => FileKind::Png,
            other => {
                return Err(Error::invalid(format!(
                    "unsupported output format `{other}` (use png or raw)"
                )))
            }
        },
        None => convert::kind_from_path(path).unwrap_or(FileKind::Png),
    };

    match kind {
        FileKind::Raw => convert::write_raw(path, image),
        FileKind::Png | FileKind::Vpng => convert::write_png(path, image),
    }
}

fn parse_codec(text: &str) -> Result<u8> {
    vpng_codecs::codec_id_by_name(text).ok_or_else(|| {
        Error::unsupported(format!(
            "codec `{}` is not implemented \
             (available: auto, raw, palette, rle, predictor, lz77, \
              predictor+dictionary, context-entropy)",
            text.to_ascii_lowercase()
        ))
    })
}

/// The set of codecs the per-tile selector may choose from.
fn parse_codec_set(text: &str) -> Result<vpng_codecs::CodecSet> {
    vpng_codecs::codec_set_by_name(text).ok_or_else(|| {
        Error::unsupported(format!(
            "codec set `{}` is not known (available: stable, extended)",
            text.to_ascii_lowercase()
        ))
    })
}

fn cmd_encode(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let output = args.require_positional(1, "output path")?.to_string();
    let limits = limits();

    let image = read_input_image(args, &input, &limits)?;

    let mut options = EncodeOptions::default().with_limits(limits);
    if let Some(edge) = args.get_u32("tile-size")? {
        if edge == 0 {
            return Err(Error::invalid("--tile-size must be non-zero"));
        }
        options = options.with_tile_size(edge, edge);
    }
    if let Some(width) = args.get_u32("tile-width")? {
        options.tile_width = width;
    }
    if let Some(height) = args.get_u32("tile-height")? {
        options.tile_height = height;
    }
    if let Some(codec) = args.get("codec") {
        options = options.with_codec(parse_codec(codec)?);
    }
    if let Some(set) = args.get("codec-set") {
        options = options.with_codec_set(parse_codec_set(set)?);
    }
    if let Some(text) = args.get("max-size") {
        options = options.with_max_size(size::parse_size(text)?);
    }
    if let Some(text) = args.get("max-size-mib") {
        let mib: u64 = text.parse().map_err(|_| {
            Error::invalid(format!("--max-size-mib expects an integer, got `{text}`"))
        })?;
        options = options.with_max_size(vpng_core::checked_mul(mib, 1024 * 1024)?);
    }

    // The v0.2 features are opt-in. Without them the encoder writes the
    // v0.1 bytes it has always written.
    if args.has("adaptive") {
        options = options.with_adaptive();
    }
    if let Some(threshold) = args.get_u32("adaptive-threshold")? {
        options = options.with_adaptive_threshold(threshold);
    }
    if args.has("pyramid") {
        let levels = match args.get_u32("pyramid-levels")? {
            Some(levels) => levels,
            None => vpng_encoder::DEFAULT_PYRAMID_LEVELS,
        };
        options = options.with_pyramid(levels);
    } else if args.get("pyramid-levels").is_some() {
        return Err(Error::invalid(
            "--pyramid-levels needs --pyramid; there is nothing to count without it",
        ));
    }

    // The GPU payload is opt-in. Without it the encoder writes the v0.1 bytes
    // it has always written.
    if args.has("gpu") {
        options = options.with_gpu();
    }

    // The streaming writer is used rather than the batch encoder, so the
    // finished file is never held in memory: it is written straight to
    // disk, through a temporary name that is renamed over the target.
    //
    // The worker count changes the speed and not the bytes: the encoder's
    // per-tile work is independent and the tiles are consumed in tile
    // order, so one worker and N workers write the same file. That is
    // checked over 144 encode cases in `vpng-encoder` and by the 16K
    // benchmark, whose SHA-256 is the proof.
    let workers = match args.get_u32("workers")? {
        Some(0) => return Err(Error::invalid("--workers must be non-zero")),
        Some(count) => count as usize,
        None => std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1),
    };
    let parallel = ParallelOptions::default().with_workers(workers);

    let report = encode_to_file(&image, &options, &parallel, std::path::Path::new(&output))?;

    let pixel_count = image.width() as u64 * image.height() as u64;
    let digest = to_hex(&canonical_digest(&image));
    if args.has("json") {
        println!(
            "{{\"output\":\"{}\",\"input\":\"{}\",\"width\":{},\"height\":{},\"format\":\"{}\",\"tiles\":{},\"levels\":{},\"file_size\":{},\"ratio\":{:.4},\"bpp\":{:.4},\"workers\":{},\"pyramid\":{},\"adaptive\":{},\"gpu\":{},\"gpu_payload_bytes\":{},\"gpu_table_bytes\":{},\"canonical_sha256\":\"{}\"}}",
            escape(&output),
            escape(&input),
            image.width(),
            image.height(),
            image.format(),
            report.tile_count,
            report.level_count,
            report.file_size,
            report.compression_ratio(),
            report.file_size as f64 / pixel_count as f64,
            workers,
            options.pyramid,
            options.adaptive,
            options.gpu,
            report.gpu_payload_bytes,
            report.gpu_table_bytes,
            digest
        );
    } else {
        println!("encoded {input} -> {output}");
        println!(
            "  {}x{} {}  {} tiles  {} bytes  ratio {:.3}x  {:.3} B/px  {} workers",
            image.width(),
            image.height(),
            image.format(),
            report.tile_count,
            report.file_size,
            report.compression_ratio(),
            report.file_size as f64 / pixel_count as f64,
            workers
        );
        if report.level_count > 1 {
            println!(
                "  {} levels (level 0 is the original; the rest are previews)",
                report.level_count
            );
        }
        if options.adaptive {
            println!("  adaptive partition: the tile sizes follow the picture's content");
        }
        if options.gpu {
            println!(
                "  GPU payloads: {} bytes in {} bytes of table (LOSSY; the lossless payload is unchanged)",
                report.gpu_payload_bytes, report.gpu_table_bytes
            );
        }
        println!("  canonical sha256 {digest}");
    }
    Ok(())
}

fn cmd_decode(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let output = args.require_positional(1, "output path")?.to_string();
    let limits = limits();

    // `--level` picks a preview level. Level 0 is the original and is the
    // default; a higher level is the downsampled preview, which is smaller
    // and cheaper to decode but is not the original.
    let level = match args.get_u32("level")? {
        Some(level) => level as usize,
        None => 0,
    };

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    if level >= document.level_count() {
        return Err(Error::invalid(format!(
            "the file has {} level(s); there is no level {level}",
            document.level_count()
        )));
    }
    let image = document.decode_level(level, &limits)?;
    write_output_image(args, &output, &image)?;

    if args.has("json") {
        println!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"level\":{},\"width\":{},\"height\":{},\"format\":\"{}\",\"canonical_sha256\":\"{}\"}}",
            escape(&input),
            escape(&output),
            level,
            image.width(),
            image.height(),
            image.format(),
            to_hex(&canonical_digest(&image))
        );
    } else {
        let kind = if level == 0 {
            "original"
        } else {
            "derived preview"
        };
        println!(
            "decoded {input} -> {output}  ({}x{} {}, level {level}, {kind})",
            image.width(),
            image.height(),
            image.format()
        );
    }
    Ok(())
}

fn cmd_info(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let limits = limits();
    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    let header: &Header = document.header();

    if args.has("json") {
        println!(
            "{{\"file\":\"{}\",\"file_size\":{},\"major\":{},\"minor\":{},\"width\":{},\"height\":{},\"color_model\":\"{}\",\"bits_per_channel\":{},\"channels\":{},\"default_codec\":{},\"tile_count\":{},\"tile_width\":{},\"tile_height\":{},\"level_count\":{},\"layer_count\":{},\"layer_table_offset\":{},\"layer_table_length\":{},\"gpu_table_offset\":{},\"gpu_table_length\":{},\"directory_offset\":{},\"declared_file_size\":{},\"feature_flags\":{},\"payloads_verified\":false}}",
            escape(&input),
            bytes.len(),
            header.major,
            header.minor,
            header.width,
            header.height,
            header.color_model.name(),
            header.bits.bits(),
            header.channels(),
            header.default_codec,
            header.tile_count,
            document.partition().nominal_tile_size().0,
            document.partition().nominal_tile_size().1,
            document.level_count(),
            document.layer_count(),
            header.layer_table_offset,
            header.layer_table_length,
            header.gpu_table_offset,
            header.gpu_table_length,
            header.directory_offset,
            header.declared_file_size,
            header.feature_flags
        );
        return Ok(());
    }

    println!("file              {input}");
    println!("file size         {} bytes", bytes.len());
    println!("format version    {}.{}", header.major, header.minor);
    println!("dimensions        {}x{}", header.width, header.height);
    println!("pixel format      {}", document.format());
    println!("channels          {}", header.channels());
    println!("default codec     {}", header.default_codec);
    println!("tile count        {}", header.tile_count);
    let (nominal_width, nominal_height) = document.partition().nominal_tile_size();
    let (typical_width, typical_height) = document.partition().typical_tile_size();
    if document.partition().is_uniform_size() {
        println!("nominal tile      {nominal_width}x{nominal_height}");
    } else {
        // An adaptive partition has no single tile size, so the first gap
        // between two origins is not a useful summary. The typical size is.
        println!(
            "typical tile      {typical_width}x{typical_height} (adaptive: the tile sizes vary)"
        );
    }
    println!("level count       {}", document.level_count());
    for (position, level) in document.levels().iter().enumerate() {
        let kind = if level.is_derived() {
            "derived preview"
        } else {
            "original"
        };
        println!(
            "  level {position}        {}x{} ({}), {} tiles",
            level.dimensions().width,
            level.dimensions().height,
            kind,
            level.tile_count()
        );
    }
    println!("directory offset  {}", header.directory_offset);
    println!("declared size     {}", header.declared_file_size);
    println!("feature flags     {:#010x}", header.feature_flags);
    if header.has_gpu() {
        println!(
            "GPU table         {} bytes at offset {}",
            header.gpu_table_length, header.gpu_table_offset
        );
    }
    if header.has_layers() {
        println!(
            "layer table       {} bytes at offset {}",
            header.layer_table_length, header.layer_table_offset
        );
        println!("layer count       {}", document.layer_count());
        for layer in document.layers() {
            let rect = layer.rect();
            println!(
                "  layer {:<2} {:<12} {:<6} {}+{} {}x{}  opacity {:<3} {} tiles  {} B -> {} B",
                layer.index(),
                layer.name(),
                if layer.is_mask() { "mask" } else { "pixels" },
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                layer.opacity(),
                layer.base_level().tile_count(),
                layer.uncompressed_bytes(),
                layer.compressed_bytes(),
            );
        }
    }
    // `info` reads only the header and the directory, which is what keeps it
    // O(1) in the tile count. It therefore cannot detect a corrupted payload.
    println!();
    println!("note: header and directory only; tile payloads were NOT verified.");
    println!("      run `vpng check {input}` for a full integrity check.");
    Ok(())
}

/// Validate a `.vpng`'s internal integrity with no reference image.
///
/// Structural validation happens in `Document::parse`; this additionally CRCs
/// every tile payload. It decodes no pixels and allocates no pixel buffer.
fn cmd_check(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let limits = limits();

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    let report = document.check_integrity()?;

    if args.has("json") {
        println!(
            "{{\"file\":\"{}\",\"file_size\":{},\"tiles_checked\":{},\"payload_bytes\":{},\"integrity\":\"ok\"}}",
            escape(&input),
            bytes.len(),
            report.tiles_checked,
            report.payload_bytes
        );
    } else {
        println!("OK    {input}");
        println!(
            "  {} tiles, {} payload bytes verified with CRC32C",
            report.tiles_checked, report.payload_bytes
        );
    }
    Ok(())
}

fn cmd_verify(args: &Args) -> Result<()> {
    let original_path = args
        .require_positional(0, "original image path")?
        .to_string();
    let candidate_path = args
        .require_positional(1, "candidate .vpng path")?
        .to_string();
    let limits = limits();

    let original = read_input_image(args, &original_path, &limits)?;

    let bytes = std::fs::read(&candidate_path)?;
    let document = Document::parse(&bytes, &limits)?;
    let decoded = document.decode_all(&limits)?;

    let original_digest = canonical_digest(&original);
    let decoded_digest = canonical_digest(&decoded);

    if original == decoded {
        println!(
            "PASS  {} == {}  (sha256 {})",
            original_path,
            candidate_path,
            to_hex(&original_digest)
        );
        Ok(())
    } else {
        Err(Error::verification(format!(
            "pixels differ: original sha256 {}, decoded sha256 {}",
            to_hex(&original_digest),
            to_hex(&decoded_digest)
        )))
    }
}

fn cmd_decode_region(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let output = args.require_positional(1, "output path")?.to_string();
    let limits = limits();

    let x = args.require_u32("x")?;
    let y = args.require_u32("y")?;
    let width = args.require_u32("width")?;
    let height = args.require_u32("height")?;
    let rect = Rect::new(x, y, width, height)?;

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    let region = document.decode_region(rect, &limits)?;
    write_output_image(args, &output, &region)?;

    if args.has("json") {
        println!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"sha256\":\"{}\"}}",
            escape(&input),
            escape(&output),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            to_hex(&canonical_digest(&region)),
        );
    } else {
        println!(
            "decoded region {}x{}+{}+{} -> {}  (sha256 {})",
            rect.width,
            rect.height,
            rect.x,
            rect.y,
            output,
            to_hex(&canonical_digest(&region))
        );
    }
    Ok(())
}

fn cmd_extract_tile(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let output = args
        .get("output")
        .map(|s| s.to_string())
        .or_else(|| args.positional(1).map(|s| s.to_string()))
        .ok_or_else(|| Error::invalid("extract-tile needs --output"))?;
    let limits = limits();

    let tile_x = args.require_u32("tile-x")?;
    let tile_y = args.require_u32("tile-y")?;
    let level = match args.get_u32("level")? {
        Some(level) => level as usize,
        None => 0,
    };

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    if level >= document.level_count() {
        return Err(Error::invalid(format!(
            "the file has {} level(s); there is no level {level}",
            document.level_count()
        )));
    }
    let level = document.level(level).expect("the level was checked above");
    let position = level
        .partition()
        .tile_at_grid_position(tile_x, tile_y)
        .ok_or_else(|| {
            Error::invalid(format!(
                "there is no tile at grid position ({tile_x}, {tile_y})"
            ))
        })?;
    let index = level
        .entry_index(position)
        .expect("a partition tile is always inside its level");
    let geom = level
        .partition()
        .tile(position)
        .expect("a partition tile is always inside its level");
    let pixels = document.decode_tile(index, &limits)?;

    let image = vpng_core::CanonicalImage::from_vec_checked(
        geom.width,
        geom.height,
        document.format(),
        pixels,
        &limits,
    )?;
    write_output_image(args, &output, &image)?;

    println!(
        "extracted tile ({tile_x}, {tile_y}) at {}+{} {}x{} -> {}",
        geom.x, geom.y, geom.width, geom.height, output
    );
    Ok(())
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Open a container over HTTP Range requests and print its summary.
///
/// The command reports the network volume as well as the container: how many
/// requests were made and how many bytes came back, next to the size of the
/// object. A reader that fetches only what it needs is visible in those two
/// numbers.
fn cmd_http_info(args: &Args) -> Result<()> {
    let url = args.require_positional(0, "url")?.to_string();
    let limits = limits();

    let source = vpng_http::HttpSource::open(&url, vpng_http::HttpSourceOptions::default())?;
    let object_size = source.length();
    let reader = vpng_source::RangeReader::open_source(source, &limits)?;

    let header: &Header = reader.meta().header();
    let tile_count = header.tile_count;

    if args.has("json") {
        println!(
            "{{\"url\":\"{}\",\"object_size\":{},\"bytes_read\":{},\"source_bytes_received\":{},\"requests\":{},\"major\":{},\"minor\":{},\"width\":{},\"height\":{},\"color_model\":\"{}\",\"bits_per_channel\":{},\"channels\":{},\"default_codec\":{},\"tile_count\":{},\"level_count\":{},\"layer_count\":{}}}",
            escape(&url),
            object_size,
            reader.bytes_read(),
            reader.source().info().bytes_received,
            reader.source().info().requests,
            header.major,
            header.minor,
            header.width,
            header.height,
            header.color_model.name(),
            header.bits.bits(),
            header.channels(),
            header.default_codec,
            tile_count,
            reader.meta().levels().len(),
            reader.meta().layer_count(),
        );
    } else {
        println!("url              {url}");
        println!("object size      {object_size} bytes");
        println!("bytes read       {} bytes", reader.bytes_read());
        println!(
            "bytes received   {} bytes (the source's own count, including the length probe)",
            reader.source().info().bytes_received
        );
        println!("requests         {}", reader.source().info().requests);
        println!(
            "format           v{}.{}  {}x{}  {}  {} bpc",
            header.major,
            header.minor,
            header.width,
            header.height,
            header.color_model.name(),
            header.bits.bits()
        );
        println!("default codec    {}", header.default_codec);
        println!("tiles            {tile_count}");
        println!("levels           {}", reader.meta().levels().len());
        println!("layers           {}", reader.meta().layer_count());
        println!();
        println!("note: the header and the directory were fetched; tile payloads were NOT.");
        println!("      run `vpng http-check {url}` to fetch and verify every payload.");
    }
    Ok(())
}

/// Decode one rectangle of a container fetched over HTTP Range requests.
fn cmd_http_decode_region(args: &Args) -> Result<()> {
    let url = args.require_positional(0, "url")?.to_string();
    let output = args.require_positional(1, "output path")?.to_string();
    let limits = limits();

    let x = args.require_u32("x")?;
    let y = args.require_u32("y")?;
    let width = args.require_u32("width")?;
    let height = args.require_u32("height")?;
    let rect = Rect::new(x, y, width, height)?;

    let source = vpng_http::HttpSource::open(&url, vpng_http::HttpSourceOptions::default())?;
    let object_size = source.length();
    let mut reader = vpng_source::RangeReader::open_source(source, &limits)?;
    let layer_count = reader.meta().layer_count();
    let region = reader.decode_region(rect, &limits)?;
    write_output_image(args, &output, &region)?;

    let digest = to_hex(&canonical_digest(&region));
    if args.has("json") {
        println!(
            "{{\"url\":\"{}\",\"output\":\"{}\",\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"sha256\":\"{}\",\"object_size\":{},\"bytes_read\":{},\"source_bytes_received\":{},\"requests\":{},\"layer_count\":{}}}",
            escape(&url),
            escape(&output),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            digest,
            object_size,
            reader.bytes_read(),
            reader.source().info().bytes_received,
            reader.source().info().requests,
            layer_count,
        );
    } else {
        println!(
            "decoded region {}x{}+{}+{} -> {}  (sha256 {})",
            rect.width, rect.height, rect.x, rect.y, output, digest
        );
        println!(
            "  fetched {} bytes of {} in {} requests",
            reader.bytes_read(),
            object_size,
            reader.source().info().requests
        );
        if layer_count > 1 {
            println!(
                "  note: the region is the BASE layer's; the file has {layer_count} layers, \
                 and the range reader does not composite them"
            );
        }
    }
    Ok(())
}

fn print_help() {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "vpng {version} — lossless, tiled, random-access image container

USAGE:
    vpng <command> [arguments] [options]

COMMANDS:
    encode <input> <output>            Encode an image into a .vpng file
    encode-layers <output>             Build a layered .vpng file (--manifest)
    decode <input.vpng> <output>       Decode a .vpng file back to an image
    decode-layer <input.vpng> <out>    Decode one layer's own pixels (--layer)
    composite <input.vpng> <output>    Blend every layer into one image
    layers <input.vpng>                List the layers of a file
    gpu-info <input.vpng>              List the file's GPU payload table
    gpu-decode <input.vpng> <out>      Decode a rectangle of the GPU payload (LOSSY)
    ui <input.vpng>                    Open the graphical editor in a browser
    info <input.vpng>                  Print header and directory summary
    check <input.vpng>                 Verify container integrity (every payload CRC32C)
    verify <original> <candidate.vpng> Fail unless pixels round-trip exactly
    decode-region <input.vpng> <out>   Decode one rectangle (--x --y --width --height)
    extract-tile <input.vpng>          Decode one tile (--tile-x --tile-y --output)
    http-info <url>                    Print the summary of a container fetched over HTTP
    http-decode-region <url> <out>     Decode one rectangle fetched over HTTP
    help                               Show this help
    version                            Show the version

OPTIONS:
    --tile-size <n>        Square tile edge for encoding (default 256)
    --tile-width <n>       Non-square tile width
    --tile-height <n>      Non-square tile height
    --codec <name>         Codec: auto (default), raw, palette, rle, predictor,
                           lz77, predictor+dictionary, context-entropy
    --codec-set <set>      Which codecs the per-tile selector may use:
                           stable (default: raw, palette, rle, predictor) or
                           extended (every codec, including the three
                           promoted in v0.3)
    --max-size <size>      Decimal byte ceiling, e.g. 10MB (ADR-0006)
    --max-size-mib <n>     Binary MiB ceiling
    --pyramid              Build a preview pyramid (a v0.2 file). The original
                           is still level 0 and is still exact.
    --pyramid-levels <n>   How many derived preview levels to build (default 3)
    --adaptive             Choose the tile sizes from the picture's content
                           (a v0.2 file). The partition still covers every
                           pixel exactly once.
    --adaptive-threshold <n>  The busyness threshold of the adaptive planner
                           (default 8: a mean neighbour difference of 8/255)
    --gpu                 Give every tile an optional LOSSY GPU payload beside
                           its lossless one (a v0.5 file). The lossless
                           payload is untouched and still reconstructs the
                           image exactly. With `encode-layers`, a mask
                           layer's tile carries its coverage alone (BC4),
                           because a mask's colour channels carry no meaning.
    --level <n>            Which level to decode (default 0, the original)
    --layer <n>            Which layer to decode (default 0, the base layer)
    --manifest <path>      The layer manifest `encode-layers` reads (JSON)
    --port <n>             The port `ui` listens on (default: a free one)
    --max-region <n>       The largest number of pixels one display request may
                           carry in `ui` (default: 4194304)
    --no-browser           `ui` prints the address instead of opening a browser
    --workers <n>          Encoder worker threads (default: the logical CPU count).
                           The worker count changes the speed and not the bytes:
                           one worker and N workers write the same file.
    --input-format <kind>  png | raw | vpng (default: by extension)
    --format <name>        Pixel format for raw I/O, e.g. rgba8, rgb16
    --width <n> --height <n>  Dimensions for raw input
    --json                 Machine-readable output
    -o, --output <path>    Output path where a positional is not used
    -h, --help             Show this help

EXIT CODES:
    0 success  1 generic  2 invalid arguments  3 unsupported feature
    4 corrupt file  5 resource limit  6 size limit  7 verification failed  8 I/O

`info` reads only the header and directory, so it is fast but is NOT an
integrity check. Use `check` to validate a file on its own, and `verify` to
compare a file against the original image.

Pixels are reproduced exactly. The 10 MB ceiling is a measured target, never a
promise: if no lossless file fits, encoding fails with exit code 6."
    );
}
