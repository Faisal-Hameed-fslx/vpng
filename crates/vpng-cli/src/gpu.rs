//! The CLI's GPU payload commands.
//!
//! A **GPU payload** is the file's optional, explicitly named, **lossy**
//! sibling of a tile's lossless payload: the same picture in the block
//! format a GPU samples as a texture (BC1 colour blocks, BC4 alpha blocks).
//!
//! Two commands cover them:
//!
//! * `vpng gpu-info` — what the file's GPU table holds;
//! * `vpng gpu-decode` — one rectangle of the GPU payload, as RGBA8.
//!
//! Neither command touches the lossless payload, and neither is part of an
//! exact operation. The lossless payload — the one that reconstructs the
//! image exactly — is what `vpng decode` reads.

use vpng_container::to_hex;
use vpng_core::{Error, Rect, Result};
use vpng_decoder::Document;

use crate::args::Args;
use crate::{escape, limits, write_output_image};

/// List what a file's GPU payload table holds.
pub fn cmd_gpu_info(args: &Args) -> Result<()> {
    let input = args.require_positional(0, "input path")?.to_string();
    let limits = limits();

    let bytes = std::fs::read(&input)?;
    let document = Document::parse(&bytes, &limits)?;

    let table = document.gpu_table().ok_or_else(|| {
        Error::unsupported(format!(
            "this file carries no GPU payload table; its minor version is {}",
            document.header().minor
        ))
    })?;

    let total_payload: u64 = table
        .records
        .iter()
        .map(|record| u64::from(record.payload_length))
        .sum();
    let with_colour = table
        .records
        .iter()
        .filter(|record| record.colour_codec != 0)
        .count();
    let with_alpha = table
        .records
        .iter()
        .filter(|record| record.alpha_codec != 0)
        .count();

    if args.has("json") {
        let mut parts = Vec::new();
        for (index, record) in table.records.iter().enumerate() {
            parts.push(format!(
                "{{\"tile\":{},\"offset\":{},\"length\":{},\"colour_codec\":{},\"alpha_codec\":{},\"present\":{}}}",
                index,
                record.payload_offset,
                record.payload_length,
                record.colour_codec,
                record.alpha_codec,
                record.is_present(),
            ));
        }
        println!(
            "{{\"file\":\"{}\",\"minor\":{},\"tile_count\":{},\"record_count\":{},\"gpu_table_offset\":{},\"gpu_table_length\":{},\"gpu_payload_bytes\":{},\"tiles_with_colour\":{},\"tiles_with_alpha\":{},\"lossy\":true,\"records\":[{}]}}",
            escape(&input),
            document.header().minor,
            document.header().tile_count,
            table.len(),
            document.header().gpu_table_offset,
            document.header().gpu_table_length,
            total_payload,
            with_colour,
            with_alpha,
            parts.join(","),
        );
        return Ok(());
    }

    println!("file              {input}");
    println!(
        "format version    {}.{}",
        document.header().major,
        document.header().minor
    );
    println!(
        "gpu table         {} bytes at offset {}",
        document.header().gpu_table_length,
        document.header().gpu_table_offset
    );
    println!("gpu records       {}", table.len());
    println!("gpu payload bytes {total_payload}");
    println!("tiles with colour {with_colour}");
    println!("tiles with alpha  {with_alpha}");
    println!();
    println!("note: the GPU payload is LOSSY. It is not used to reconstruct the image.");
    println!("      run `vpng decode` for the exact pixels.");
    Ok(())
}

/// Decode one rectangle of a file's GPU payload.
///
/// The output is always RGBA8: a GPU payload is always eight-bit, and a
/// texture is always RGBA.
pub fn cmd_gpu_decode(args: &Args) -> Result<()> {
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
    if document.gpu_table().is_none() {
        return Err(Error::unsupported("this file carries no GPU payload table"));
    }

    let region = document.decode_gpu_region(rect, &limits)?;
    write_output_image(args, &output, &region)?;

    if args.has("json") {
        println!(
            "{{\"input\":\"{}\",\"output\":\"{}\",\"x\":{},\"y\":{},\"width\":{},\"height\":{},\"format\":\"{}\",\"sha256\":\"{}\",\"lossy\":true}}",
            escape(&input),
            escape(&output),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            region.format(),
            to_hex(&vpng_container::canonical_digest(&region)),
        );
    } else {
        println!(
            "decoded the GPU payload {}x{}+{}+{} -> {output}  (RGBA8, LOSSY)",
            rect.width, rect.height, rect.x, rect.y
        );
        println!(
            "  sha256 {}",
            to_hex(&vpng_container::canonical_digest(&region))
        );
    }
    Ok(())
}
