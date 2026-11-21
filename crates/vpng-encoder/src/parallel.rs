//! Parallel encoding: the same file, made by more than one thread.
//!
//! The encoder's per-tile work is independent. [`select_payload`] sees only
//! the tile's own pixels, its own geometry and the encode options, so the
//! same tile always produces the same payload no matter which thread encodes
//! it or in what order the threads finish. That is what makes parallel
//! encoding safe.
//!
//! The rule this module must never break:
//!
//! > One worker and N workers produce the same file, byte for byte.
//!
//! Two things make that hold by construction rather than by luck:
//!
//! * **Assembly is shared.** The serial encoder, the parallel encoder and
//!   the streaming writer all build the container the same way, so there is
//!   only one place where offsets and checksums are computed.
//! * **Tiles are consumed in tile order.** Workers finish out of order, but
//!   the coordinator hands the tiles to the sink in index order, so the
//!   payloads are laid out exactly as the serial encoder lays them out.
//!
//! Memory is bounded by the number of chunks in flight, not by the size of
//! the image, which is what lets the streaming writer run in parallel too.
//!
//! The work is described by a [`Partition`], not by a grid, so a file with
//! variable-size tiles is encoded by exactly the same code as one with a
//! uniform grid.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};

use vpng_codecs::EncodeContext;
use vpng_core::{Dimensions, Error, Result};
use vpng_tiling::{Partition, TileRect};

use crate::source::PixelSource;
use crate::{select_payload, EncodeOptions, EncodeReport, TilePayload};

/// How the parallel encoder is run.
#[derive(Debug, Clone, Copy)]
pub struct ParallelOptions {
    /// The number of worker threads. One means "run inline", with no threads
    /// spawned at all, which is what the batch encoder uses.
    pub workers: usize,
    /// How many consecutive tiles one worker encodes before handing them over.
    ///
    /// A chunk is the unit of work, not the tile. Bigger chunks mean less
    /// coordination; smaller chunks mean better balance when some tiles are
    /// much more expensive than others.
    pub chunk_tiles: usize,
}

impl Default for ParallelOptions {
    fn default() -> Self {
        ParallelOptions {
            workers: 1,
            chunk_tiles: 4,
        }
    }
}

impl ParallelOptions {
    /// Use `workers` threads.
    pub fn with_workers(mut self, workers: usize) -> Self {
        self.workers = workers.max(1);
        self
    }

    /// Encode `chunk_tiles` tiles per work item.
    pub fn with_chunk_tiles(mut self, chunk_tiles: usize) -> Self {
        self.chunk_tiles = chunk_tiles.max(1);
        self
    }
}

/// Encode a pixel source into a complete `.vpng` byte stream.
///
/// The result is byte-identical to [`crate::encode`] for the same pixels and
/// the same options, whatever `parallel.workers` is.
pub fn encode_parallel(
    source: &dyn PixelSource,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
) -> Result<EncodeReport> {
    let dims = Dimensions {
        width: source.width(),
        height: source.height(),
    };

    // The adaptive planner and the pyramid builder both need the picture's
    // content, so a source that cannot be asked for "some pixels" is read
    // into a whole image first. The default path never does that.
    let (materialized, plan) = if options.is_v2() {
        let image = crate::materialize(source, options)?;
        let plan = crate::plan_file(&image, options)?;
        (Some(image), plan)
    } else {
        (None, crate::plan_uniform(dims, options)?)
    };

    let level0: &dyn PixelSource = match &materialized {
        Some(image) => image,
        None => source,
    };
    let levels = crate::encode_plan(level0, &plan, options, parallel)?;
    crate::assemble(dims, source.format(), options, &levels)
}

/// Encode every tile into one vector, in tile order.
pub(crate) fn encode_all_tiles(
    source: &dyn PixelSource,
    partition: &Partition,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
) -> Result<Vec<TilePayload>> {
    let mut payloads = Vec::with_capacity(partition.len());
    encode_tiles_in_order(source, partition, options, parallel, &mut |_, payload| {
        payloads.push(payload);
        Ok(())
    })?;
    Ok(payloads)
}

/// The state the workers and the coordinator share.
struct Shared {
    /// The next chunk a worker may take.
    next_take: usize,
    /// The next chunk the coordinator will consume.
    next_consume: usize,
    /// The chunks that have been encoded and are waiting to be consumed.
    results: HashMap<usize, Result<Vec<TilePayload>>>,
    /// Set once the coordinator is done, so the workers stop waiting.
    shutdown: bool,
}

/// Encode every tile, handing the finished tiles to `sink` in tile order.
///
/// `sink` owns each payload it is handed, so the streaming writer can write
/// the bytes and drop them without holding the whole file in memory.
pub(crate) fn encode_tiles_in_order(
    source: &dyn PixelSource,
    partition: &Partition,
    options: &EncodeOptions,
    parallel: &ParallelOptions,
    sink: &mut dyn FnMut(usize, TilePayload) -> Result<()>,
) -> Result<()> {
    let tile_count = partition.len();
    if tile_count == 0 {
        return Ok(());
    }

    let workers = parallel.workers.max(1).min(tile_count);
    if workers <= 1 {
        // The serial path: no threads, no locks, exactly the order the batch
        // encoder has always used. The scratch buffer is reused for every
        // tile, so the encoder allocates once rather than once per tile.
        let mut pixels = Vec::new();
        for index in 0..tile_count {
            let rect = partition
                .tile(index)
                .ok_or_else(|| Error::internal(format!("tile {index} is out of range")))?;
            let payload = encode_tile(source, index, rect, options, &mut pixels)?;
            sink(index, payload)?;
        }
        return Ok(());
    }

    let chunk_tiles = parallel.chunk_tiles.max(1);
    let chunk_count = tile_count.div_ceil(chunk_tiles);
    // How many chunks may be encoded before the coordinator has caught up.
    // This is the memory bound: the workers cannot run away and buffer the
    // whole file while a slow sink is still writing.
    let max_in_flight = workers * 2;

    let shared = Mutex::new(Shared {
        next_take: 0,
        next_consume: 0,
        results: HashMap::new(),
        shutdown: false,
    });
    let job_ready = Condvar::new();
    let result_ready = Condvar::new();

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                // One scratch buffer per worker, reused for every tile it
                // encodes. The encoder allocates once per worker rather than
                // once per tile.
                let mut pixels = Vec::new();

                let chunk = {
                    let mut state = shared.lock().unwrap();
                    // Wait until there is room for one more chunk in flight.
                    while state.next_take >= state.next_consume + max_in_flight && !state.shutdown {
                        state = job_ready.wait(state).unwrap();
                    }
                    if state.shutdown {
                        return;
                    }
                    let chunk = state.next_take;
                    state.next_take += 1;
                    chunk
                };

                if chunk >= chunk_count {
                    return;
                }

                let start = chunk * chunk_tiles;
                let end = (start + chunk_tiles).min(tile_count);
                let outcome = encode_chunk(source, partition, options, start, end, &mut pixels);

                let mut state = shared.lock().unwrap();
                state.results.insert(chunk, outcome);
                drop(state);
                result_ready.notify_all();
            });
        }

        // The coordinator. It runs on the calling thread while the workers
        // run, so chunks are consumed as they are produced instead of being
        // accumulated until the workers finish.
        let outcome: Result<()> = (|| {
            for chunk in 0..chunk_count {
                let result = {
                    let mut state = shared.lock().unwrap();
                    loop {
                        if let Some(result) = state.results.remove(&chunk) {
                            state.next_consume += 1;
                            break result;
                        }
                        state = result_ready.wait(state).unwrap();
                    }
                };
                // Let the workers fill the space this chunk freed before the
                // sink runs, so a slow write does not stall the encoders.
                job_ready.notify_all();

                let payloads = result?;
                for (offset, payload) in payloads.into_iter().enumerate() {
                    sink(chunk * chunk_tiles + offset, payload)?;
                }
            }
            Ok(())
        })();

        // Whatever happened — success, an encode failure or a sink failure —
        // the workers must be woken and told to stop, or the scope would wait
        // for them forever.
        shared.lock().unwrap().shutdown = true;
        job_ready.notify_all();

        outcome
    })
}

/// Encode the tiles `start..end` of the partition, in order.
fn encode_chunk(
    source: &dyn PixelSource,
    partition: &Partition,
    options: &EncodeOptions,
    start: usize,
    end: usize,
    pixels: &mut Vec<u8>,
) -> Result<Vec<TilePayload>> {
    let mut payloads = Vec::with_capacity(end - start);
    for index in start..end {
        let rect = partition
            .tile(index)
            .ok_or_else(|| Error::internal(format!("tile {index} is out of range")))?;
        payloads.push(encode_tile(source, index, rect, options, pixels)?);
    }
    Ok(payloads)
}

/// Encode one tile. This is the whole of the encoder's per-tile work, and it
/// is the same function the serial path uses.
fn encode_tile(
    source: &dyn PixelSource,
    index: usize,
    rect: &TileRect,
    options: &EncodeOptions,
    pixels: &mut Vec<u8>,
) -> Result<TilePayload> {
    source.tile_pixels(index, rect, pixels)?;
    let uncompressed_length = u32::try_from(pixels.len())
        .map_err(|_| Error::resource_limit("tile uncompressed size exceeds u32"))?;

    // The context carries the tile's *actual* geometry: a tile in the last
    // column or row is smaller than the nominal grid, and a codec that needs
    // the pixel width (the predictor) must see the real one.
    let ctx = EncodeContext {
        limits: options.limits,
        format: source.format(),
        tile_width: rect.width as u16,
        tile_height: rect.height as u16,
    };

    let (codec_id, bytes) = select_payload(pixels.as_slice(), &ctx, options)?;
    Ok(TilePayload {
        codec_id,
        bytes,
        uncompressed_length,
    })
}
