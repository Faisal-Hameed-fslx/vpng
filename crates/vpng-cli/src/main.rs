//! `vpng` — the VPNG command line tool.
//!
//! Exit codes are stable and documented in `docs/VPNG_SPEC_0_1.md`:
//! 0 success, 1 generic, 2 invalid arguments, 3 unsupported feature,
//! 4 corrupt file, 5 resource limit, 6 size limit, 7 verification failure,
//! 8 I/O failure.

#![forbid(unsafe_code)]

mod args;
mod convert;
mod size;

use std::process::ExitCode;

use args::Args;
use convert::FileKind;
use vpng_container::{canonical_digest, to_hex, Header};
use vpng_core::{Error, Limits, Rect, Result};
use vpng_decoder::Document;
use vpng_encoder::parallel::ParallelOptions;
use vpng_encoder::stream::encode_to_file;
use vpng_encoder::EncodeOptions;

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
        "decode" => cmd_decode(&args),
        "info" => cmd_info(&args),
        "check" => cmd_check(&args),
        "verify" => cmd_verify(&args),
        "decode-region" => cmd_decode_region(&args),
        "extract-tile" => cmd_extract_tile(&args),
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
    match text.to_ascii_lowercase().as_str() {
        "auto" => Ok(vpng_codecs::CODEC_AUTO),
        "raw" | "0" => Ok(vpng_codecs::CODEC_RAW),
        "palette" | "1" => Ok(vpng_codecs::CODEC_PALETTE),
        "rle" | "2" => Ok(vpng_codecs::CODEC_RLE),
        "predictor" | "3" => Ok(vpng_codecs::CODEC_PREDICTOR),
        other => Err(Error::unsupported(format!(
            "codec `{other}` is not implemented in v0.1 \
             (available: auto, raw, palette, rle, predictor)"
        ))),
    }
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
    if let Some(text) = args.get("max-size") {
        options = options.with_max_size(size::parse_size(text)?);
    }
    if let Some(text) = args.get("max-size-mib") {
        let mib: u64 = text.parse().map_err(|_| {
            Error::invalid(format!("--max-size-mib expects an integer, got `{text}`"))
        })?;
        options = options.with_max_size(vpng_core::checked_mul(mib, 1024 * 1024)?);
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
            "{{\"output\":\"{}\",\"input\":\"{}\",\"width\":{},\"height\":{},\"format\":\"{}\",\"tiles\":{},\"file_size\":{},\"ratio\":{:.4},\"bpp\":{:.4},\"workers\":{},\"canonical_sha256\":\"{}\"}}",
            escape(&output),
            escape(&input),
            image.width(),
            image.height(),
            image.format(),
            report.tile_count,
            report.file_size,
            report.compression_ratio(),
            report.file_size as f64 / pixel_count as f64,
            workers,
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
        println!("  canonical sha256 {digest}");
    }
    Ok(())
}

fn cmd_decode(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let output = args.require_positional(1, "output path")?.to_string();
    let limits = limits();

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    let image = document.decode_all(&limits)?;
    write_output_image(args, &output, &image)?;

    if args.has("json") {
        println!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"width\":{},\"height\":{},\"format\":\"{}\",\"canonical_sha256\":\"{}\"}}",
            escape(&input),
            escape(&output),
            image.width(),
            image.height(),
            image.format(),
            to_hex(&canonical_digest(&image))
        );
    } else {
        println!(
            "decoded {input} -> {output}  ({}x{} {})",
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
            "{{\"file\":\"{}\",\"file_size\":{},\"major\":{},\"minor\":{},\"width\":{},\"height\":{},\"color_model\":\"{}\",\"bits_per_channel\":{},\"channels\":{},\"default_codec\":{},\"tile_count\":{},\"tile_width\":{},\"tile_height\":{},\"directory_offset\":{},\"declared_file_size\":{},\"feature_flags\":{},\"payloads_verified\":false}}",
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
            document.grid().tile_width(),
            document.grid().tile_height(),
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
    println!(
        "nominal tile      {}x{}",
        document.grid().tile_width(),
        document.grid().tile_height()
    );
    println!("directory offset  {}", header.directory_offset);
    println!("declared size     {}", header.declared_file_size);
    println!("feature flags     {:#010x}", header.feature_flags);
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

    println!(
        "decoded region {}x{}+{}+{} -> {}  (sha256 {})",
        rect.width,
        rect.height,
        rect.x,
        rect.y,
        output,
        to_hex(&canonical_digest(&region))
    );
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

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;
    let index = document.grid().index(tile_x, tile_y)?;
    let geom = document.grid().geometry(tile_x, tile_y)?;
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

fn print_help() {
    let version = env!("CARGO_PKG_VERSION");
    println!(
        "vpng {version} — lossless, tiled, random-access image container

USAGE:
    vpng <command> [arguments] [options]

COMMANDS:
    encode <input> <output>            Encode an image into a .vpng file
    decode <input.vpng> <output>       Decode a .vpng file back to an image
    info <input.vpng>                  Print header and directory summary
    check <input.vpng>                 Verify container integrity (every payload CRC32C)
    verify <original> <candidate.vpng> Fail unless pixels round-trip exactly
    decode-region <input.vpng> <out>   Decode one rectangle (--x --y --width --height)
    extract-tile <input.vpng>          Decode one tile (--tile-x --tile-y --output)
    help                               Show this help
    version                            Show the version

OPTIONS:
    --tile-size <n>        Square tile edge for encoding (default 256)
    --tile-width <n>       Non-square tile width
    --tile-height <n>      Non-square tile height
    --codec <name>         Codec: auto (default), raw, palette, rle, predictor
    --max-size <size>      Decimal byte ceiling, e.g. 10MB (ADR-0006)
    --max-size-mib <n>     Binary MiB ceiling
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
