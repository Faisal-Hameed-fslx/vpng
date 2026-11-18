# P4 — Memory validation report

**Gate:** P4 memory validation (16K localized editing) — **PASS**
**Date:** 2026-11-16
**Toolchain:** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `x86_64-pc-windows-gnu`, release profile
**Raw data:** `benchmarks/p4_memory_results.json`
**Harness:** `crates/vpng-validation/` (binary `vpng-memory`), driver `benchmarks/memory/run_memory.py`

> This report is part of the P3.5 + P4 milestone. The editing engine itself
> is reported in `docs/P4_EDITING_ENGINE_REPORT.md`; the format's
> compatibility is reported in `docs/P4_COMPATIBILITY_REPORT.md`.

---

## 1. What was validated

A localized edit on images up to 16384×16384, in four pixel formats, with
the target of **≤ 64 MB of CPU working memory**. The matrix is 3
resolutions × 4 formats = **12 cases**, and every case ran to completion on
this host.

| | |
|---|---|
| Resolutions | 4096², 8192², 16384² |
| Formats | `gray8` (1 B/px), `rgb8` (3 B/px), `rgba8` (4 B/px), `rgba16` (8 B/px) |
| Largest case | 16384² RGBA16 = **2,147,483,648 bytes** of canonical pixels |
| Tile size | 512 × 512 |
| Cache budget | 16 MiB |
| History budget | 32 MiB |
| Edit | a 64×64 region at the centre of the image |
| Target | ≤ 64 MB |

**Verdict: PASS 12, FAIL 0, BLOCKED 0.**

---

## 2. How the memory is measured

Two independent figures are recorded for every phase, because neither alone
tells the truth about a process.

### Heap peak — what the library used

A tracking global allocator (`vpng_validation::mem::Tracking`) counts every
allocation and deallocation. The figure is the number of bytes the process's
allocator had **outstanding** at the busiest moment.

* **Includes:** every `Vec`, `Box`, `String` and other heap allocation the
  process makes — the decoded tiles, the dirty tiles, the region buffers,
  the history snapshots, the tile payloads the writer read, the directory
  entries.
* **Excludes:** the binary itself, the stacks, the operating system's own
  bookkeeping, and the memory the operating system spends on the file's
  page cache.

This is the library's own working set, with no operating-system overhead in
it.

### Resident-set peak — what the process cost

`K32GetProcessMemoryInfo` (`PROCESS_MEMORY_COUNTERS_EX`) reports the
process's resident-set size and its high-water mark.

* **Includes:** everything the heap peak includes, **plus** the binary, the
  stacks, the allocator's own metadata and the operating system's overhead.
* **Excludes:** the file on disk, the operating system's page cache, the
  memory other processes use, and the space the temporary file occupies.

The **edit's resident-set delta** is the resident-set high-water mark minus
the resident set measured immediately before the edit began. It is what the
edit itself cost the process.

### The target is evaluated on both

The 64 MB target is checked against the edit's heap peak **and** against the
edit's resident-set delta. A case passes only if both are inside the
target, the streamed file verifies, the edited region reads back exactly, and
every untouched tile's payload is byte-identical.

---

## 3. Results

### 3.1 The 16K cases

| Case | Canonical | File on disk | Write | Write heap peak | Write RSS peak | Edit RSS Δ | Edit heap peak | Decoded tiles |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 16384² `gray8` | 256 MB | 39.8 MB | 30.9 s | 1.74 MB | 5.73 MB | 2.98 MB | 2.16 MB | 1 |
| 16384² `rgb8` | 768 MB | 110.2 MB | 84.0 s | 4.66 MB | 8.92 MB | 4.97 MB | 5.80 MB | 1 |
| 16384² `rgba8` | 1,024 MB | 144.6 MB | 113.0 s | 6.02 MB | 10.07 MB | 8.76 MB | 7.72 MB | 1 |
| 16384² `rgba16` | **2,048 MB** | 163.3 MB | 199.5 s | 8.83 MB | 13.15 MB | **13.16 MB** | **12.52 MB** | 1 |

### 3.2 The whole matrix

| Case | Canonical | File | Write | Write heap | Write RSS | Edit RSS Δ | Edit heap | Edit RSS peak | Decoded | Dirty | Copied / rewritten |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 4096² `gray8` | 16 MB | 2.4 MB | 2.2 s | 1.69 MB | 4.91 MB | 2.14 MB | 2.12 MB | 5.43 MB | 1 | 1 | 63 / 1 |
| 4096² `rgb8` | 48 MB | 6.8 MB | 6.0 s | 4.59 MB | 7.87 MB | 5.39 MB | 5.76 MB | 9.46 MB | 1 | 1 | 63 / 1 |
| 4096² `rgba8` | 64 MB | 8.9 MB | 7.6 s | 5.94 MB | 9.89 MB | 8.12 MB | 7.67 MB | 11.93 MB | 1 | 1 | 63 / 1 |
| 4096² `rgba16` | 128 MB | 10.0 MB | 12.9 s | 8.75 MB | 12.55 MB | 12.33 MB | 12.48 MB | 15.89 MB | 1 | 1 | 63 / 1 |
| 8192² `gray8` | 64 MB | 9.8 MB | 9.4 s | 1.70 MB | 5.07 MB | 2.73 MB | 2.12 MB | 6.03 MB | 1 | 1 | 255 / 1 |
| 8192² `rgb8` | 192 MB | 27.4 MB | 24.2 s | 4.61 MB | 7.94 MB | 3.85 MB | 5.77 MB | 9.48 MB | 1 | 1 | 255 / 1 |
| 8192² `rgba8` | 256 MB | 35.9 MB | 31.1 s | 5.97 MB | 9.74 MB | 7.62 MB | 7.68 MB | 11.05 MB | 1 | 1 | 255 / 1 |
| 8192² `rgba16` | 512 MB | 40.5 MB | 68.3 s | 8.78 MB | 12.66 MB | 13.12 MB | 12.49 MB | 16.50 MB | 1 | 1 | 255 / 1 |
| 16384² `gray8` | 256 MB | 39.8 MB | 30.9 s | 1.74 MB | 5.73 MB | 2.98 MB | 2.16 MB | 6.26 MB | 1 | 1 | 1023 / 1 |
| 16384² `rgb8` | 768 MB | 110.2 MB | 84.0 s | 4.66 MB | 8.92 MB | 4.97 MB | 5.80 MB | 10.66 MB | 1 | 1 | 1023 / 1 |
| 16384² `rgba8` | 1,024 MB | 144.6 MB | 113.0 s | 6.02 MB | 10.07 MB | 8.76 MB | 7.72 MB | 12.38 MB | 1 | 1 | 1023 / 1 |
| 16384² `rgba16` | **2,048 MB** | 163.3 MB | 199.5 s | 8.83 MB | 13.15 MB | **13.16 MB** | **12.52 MB** | 16.54 MB | 1 | 1 | 1023 / 1 |

**The worst case in the matrix is 16384² RGBA16, and it uses 13.16 MB of
resident-set growth and 12.52 MB of heap — 21% and 20% of the 64 MB
target.**

The memory figures are essentially **flat across the resolutions**: the
4096² RGBA16 case and the 16384² RGBA16 case use the same 12.5 MB of heap,
even though the second image's pixels are **16× larger**. That is the
measurement that matters: the engine's memory is a function of the tile
size and the budgets, not of the image's size.

### 3.3 The memory model

For a 512×512 tile:

| Structure | Bytes | Note |
|---|---:|---|
| One decoded tile (RGBA16) | 2,097,152 | 512 × 512 × 8 |
| Cache budget | 16,777,216 | 8 tiles' worth |
| History budget | 33,554,432 | 16 tiles' worth |
| Dirty tile | 2,097,152 | one tile's pixels |
| Region buffer | 65,536 | 64 × 64 × 16 (the region plus its patch) |
| **Observed peak** | **13,126,070** | 12.5 MiB |

The observed peak is far below the sum of the budgets because the edit
touches **one** tile: the cache holds one tile, the history holds one step,
and the dirty set holds one tile. The budgets are ceilings, not
reservations.

---

## 4. The twelve questions

**Q1 — Was the matrix actually run, or blocked?**
**Run: 12 of 12 cases PASS, 0 FAIL, 0 BLOCKED.** Every case ran in its own
process, wrote its file, verified it, edited it, committed it and verified
the result. No case was skipped and no case was reported as blocked. The
host had 7,916 MB of physical memory (677 MB free at the start), 6.2 GB
of free disk on `E:`, and 12 logical CPUs.

**Q2 — Does the writer avoid allocating the whole image?**
**Yes.** The streaming writer (`vpng_validation::stream::write_streaming`)
generates one tile's pixels, encodes it, writes it and moves on. The
canonical size of the image is recorded for comparison only; it is never
allocated. The evidence is the write phase's own figures: writing a
2,048 MB image costs a **8.83 MB heap peak** and a **13.15 MB resident-set
peak**. A writer that held the image would have needed 2 GB.

**Q3 — Does the edit avoid allocating the whole image?**
**Yes.** The edit decoded **exactly 1 tile** in every one of the 12 cases,
out of 64, 256 and 1,024 tiles respectively. Its resident-set growth never
exceeded 13.16 MB, on an image whose canonical size ranges from 16 MB to
2,048 MB.

**Q4 — What are the peak resident-set and heap figures per case?**
**Given in §3.2, column by column.** The maxima across the matrix: write
heap peak 8.83 MB, write resident-set peak 13.15 MB, edit heap peak
12.52 MB, edit resident-set peak 16.54 MB, edit resident-set **growth**
13.16 MB (16384² RGBA16) and 13.12 MB (8192² RGBA16).

**Q5 — Is the ≤ 64 MB target met?**
**Yes, on every case, on both measures.** The largest edit's heap peak is
13,126,070 bytes (12.5 MiB) and the largest edit's resident-set growth is
13,799,424 bytes (13.2 MiB) — both well under 67,108,864 bytes (64 MiB).
The driver fails a case if either figure exceeds the target.

**Q6 — What is included in, and excluded from, the reported memory?**
**Included in the heap peak:** the decoded-tile cache, the undo history's
snapshots, the dirty tiles' pixels, the region buffers, the tile payloads the
writer read, the directory entries, and the JSON report being assembled.
**Included in the resident-set peak:** all of the above, plus the binary,
the stacks, the allocator's metadata and the operating system's overhead.
**Excluded from both:** the operating system's page cache, the file on disk,
the memory other processes use, and the space the temporary file occupies
during the commit. These exclusions are recorded in the
`definitions` section of `benchmarks/p4_memory_results.json`.

**Q7 — How many tiles were decoded, and how many were re-encoded?**
**One tile decoded, one tile rewritten, in all 12 cases.** The commit
reports 63 / 255 / 1023 tiles copied verbatim and 1 tile rewritten, so
99.90% of the 16K file's payloads are reused byte for byte.

The four counts are distinct and are reported separately (see
`docs/P4_EDITING_ENGINE_REPORT.md` §3.0): **decoded** (1 — the tile the
edit read), **modified** (1 — the tile whose pixels changed), **rewritten**
(1 — the tile whose encoded payload the commit replaced), and **preserved**
(63 / 255 / 1023 — the tiles whose original payloads were copied byte for
byte). Decoding is not rewriting: the commit re-encodes the modified tile
from the pixels the dirty set already holds, so the commit itself decodes
no tile at all.

**Q8 — Do the cache and the history stay inside their budgets?**
**Yes.** `cache.within_budget` and `history.within_budget` are true in
every case. The cache peak is 1.00 MB (RGBA8) to 2.00 MB (RGBA16) against
a 16 MB budget; the history peak is the same, against a 32 MB budget. The
benchmark's dedicated budget scenarios, in
`benchmarks/p4_editing_results.json`, additionally show a cache filling to
*exactly* its 524,288-byte budget with 2 evictions, and a history that
retains 3 of 12 steps and drops 9 within a 1,048,576-byte budget.

**Q9 — How is the result verified, and not merely asserted?**
**Three independent checks per case.**
1. The streamed file is reopened with the ordinary decoder, its container
   integrity is checked, and three tiles (the first, a middle one and the
   last) are decoded and compared byte for byte against the pattern the
   writer generated. `verification.mismatches` is 0 in every case.
2. The edited region is read back from the committed file and compared byte
   for byte with the patch that was written.
   `pixels_read_back_exact` is true in every case.
3. The CRC32C of every tile payload is recorded before and after the save;
   exactly the dirty tile's CRC changes.
   `untouched_payloads_byte_identical` is true in every case.

**Q10 — Is the result reproducible?**
**Yes.** Each case runs in its own process, so the process's resident-set
high-water mark belongs to that case alone. The measurement command, the
tile size, the budgets and the host's memory and disk figures are recorded
in `benchmarks/p4_memory_results.json` alongside the results. The driver
is deterministic: the image pattern is a pure function of the coordinates,
with no clock and no random source. The one non-deterministic element is
the wall-clock timing, which is reported as a measurement, not as a pass
condition.

**Q11 — What are the host's facts?**
| | |
|---|---|
| Platform | `win32`, `x86_64-pc-windows-gnu` |
| Logical CPUs | 12 |
| Physical memory | 7,916 MB total, 677 MB free at the start of the matrix |
| Work disk (`E:`) | 6.2 GB free |
| `C:` | 3.2 GB free |
| Toolchain | `rustc 1.100.0`, release profile |
| Python | the driver script's own version, recorded in the report |

The host is deliberately modest: 8 GB of RAM with 0.7 GB free is *less*
than the 2 GB canonical size of the largest case, which is why the
streaming writer matters.

**Q12 — What is explicitly not claimed?**
* **No claim that the memory figure is small in the abstract.** The report
  gives the numbers and the model behind them; it does not call them
  "efficient".
* **No claim about a different host.** These figures are for this machine.
  A machine with a different allocator, a different page size or a
  different file system will produce different numbers.
* **No claim about the *peak* of a whole editing session.** The matrix
  measures one localized edit in a fresh process. A session that edits
  many tiles over time is bounded by the cache and history budgets, but
  that session's peak was not measured here.
* **No claim about a lossy path.** There is none: every case is lossless,
  and the verification is byte-exact.
* **No claim about 32K or larger images.** The matrix stops at 16384².
  Larger images are not covered by this report, and the disk space they
  would need was not available to demonstrate it.
* **No claim that the write timings are fast.** They are reported as
  measurements on this host. The 199.5-second write of the 16K RGBA16 case
  is dominated by the predictor codec's arithmetic coding; no optimisation
  claim is made.

---

## 5. Reproducing the evidence

```powershell
cargo build --release --workspace
python benchmarks/memory/run_memory.py
```

The driver writes `benchmarks/p4_memory_results.json`. A single case can be
re-run with:

```powershell
python benchmarks/memory/run_memory.py --only 16384x16384-rgba16
```

Each case is a separate `vpng-memory` process, so a partial matrix can be
resumed without losing the results of the cases that already passed.

---

## 6. A note on the measurements' limits

The resident-set high-water mark is a *process* figure: it never goes down
within the life of a process. The driver therefore runs one case per
process rather than resetting the mark, because a reset would leave the
earlier phases' peak in the later phases' figure. The price is that each
case pays the process's start-up cost (the binary, the runtime and the
allocator's initial arena) inside its own peak — which is the conservative
choice, because that cost is real for any application that edits an image.
