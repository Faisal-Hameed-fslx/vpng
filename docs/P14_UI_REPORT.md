# P14 — the graphical editor

**Date:** 2026-11-28
**Deliverables:** the editing session in `crates/vpng-ui/src/session.rs`; the
protocol in `crates/vpng-ui/src/protocol.rs`; the HTTP/1.1 server in
`crates/vpng-ui/src/http.rs`; the routes and the trust model in
`crates/vpng-ui/src/server.rs`; the embedded window in
`crates/vpng-ui/src/assets.rs`; the front end in `ui/`; the command line's
`vpng ui` in `crates/vpng-cli/src/ui.rs`; the shared strict JSON reader and
writer in `crates/vpng-json`; the measurement drivers
`benchmarks/ui/run_ui_corpus.py` and `benchmarks/ui/measure_ui_memory.py`;
the raw result files `benchmarks/p14_ui_results.json` and
`benchmarks/p14_ui_memory_results.json`; and the decision record
`docs/decisions/ADR-0020-graphical-editor.md`.
**Status:** the case **ran** — it did not have to be reported BLOCKED. Every
figure below is read out of the raw JSON, and `check_ui()` in
`benchmarks/verify_reports.py` binds the quoted figures to it.

---

## 1. What this is

Phase C step 6 of the agreed plan: **the graphical editor**.

`vpng ui <file.vpng>` opens the file in a graphical editor. The editor is a
**local program** with a **browser** front end: it serves a small web
application on the loopback interface and opens it in the browser.

### 1.1 The window is a browser, and why

A native toolkit — egui, Slint, iced, GTK — was considered and rejected.
The reason is not taste; it is three properties of this repository:

* **The workspace's minimum Rust version is 1.74.** The current graphical
  toolkits require a newer compiler. Raising the minimum version for a
  window would be a cost paid by every consumer of the library for the
  benefit of one program.
* **The workspace deliberately has no windowing stack.** No asynchronous
  runtime, no web framework, no event loop, no drawing library. A native
  window would add all of them.
* **The continuous-integration job has no display server.** A browser front
  end can be tested — the editor's whole surface is HTTP and JSON — while a
  native window cannot be tested at all.

So the editor is a **view and a set of commands**. It draws bytes the
engine sends and sends commands the engine executes. There is no image codec
in JavaScript, and no way for the page to write a file.

### 1.2 The editor is a client of the format

The editor introduces **no format revision**: no new header field, no new
directory flag, no new payload kind, and no change to any published byte.
It reads and writes through the existing engines, so the guarantees the
editing engine already has are the editor's guarantees:

* a save rewrites **only** the tiles the edit dirtied, and copies every
  other tile's payload byte for byte;
* the commit writes a temporary file and renames it over the target, so a
  failure leaves the original untouched;
* an undo restores the previous pixels exactly, because the history holds the
  pixels rather than a picture of them.

### 1.3 One decoder, not two

`vpng-view` decides which tiles are on screen and which are wanted next:
`Viewport::plan` yields the visible tiles and the prefetch ring. The
**prefetch is carried out by the editing engine**, one tile at a time, so
the transient buffer is one tile rather than the whole ring and there is
still **one decoder** in the process.

### 1.4 The editor is local-only

The editor has **no password** and it can write the file it opens. It is
therefore bound to the loopback interface and to nothing else, and it checks
two things before it answers a request that could change anything:

* the **`Host` header** must name a loopback address, so a web page cannot
  reach the editor through a name that resolves to the machine — the
  DNS-rebinding attack;
* the **session token** must be presented in the `X-Vpng-Token` header. The
  token comes from the operating system's random source and is printed in
  the address the editor opens.

The static assets — the page, the script, the stylesheet, the geometry
module — are served without the token, because they are text and carry no
authority.

---

## 2. How it was measured

### 2.1 The host

Intel Core i5-1235U, 12 logical processors, 7.7 GB physical RAM, Windows
`x86_64-pc-windows-gnu`, `rustc 1.100.0`, release build, tile size 256,
four encoder workers.

### 2.2 The corpus

The same 750-image corpus every other milestone used: 300 photographs, 100
screenshots, 100 pixel-art images, 100 scans and 150 textures, with a
SHA-256 manifest (`benchmarks/corpus_manifest.json`).

### 2.3 The method

The measurement drives the **real `vpng ui` program** over HTTP. The client
is the Python standard library's `urllib` — no third-party package is
installed — and it speaks the editor's protocol: JSON in, JSON out, with the
picture's pixels coming back as raw bytes.

The checker is **not** the project's own decoder. The container is read
field by field from the **specification** — the header's fields at their
documented offsets, the 40-byte directory entries at theirs — and the
display conversion is re-implemented from the ADR's rules. A bug in the
editor therefore cannot hide behind a matching bug in the checker.

For every image the driver:

1. encodes the canonical pixels into a plain v0.1 file with the real
   command line;
2. starts `vpng ui` over that file and reads the address it prints;
3. reads the editor's **state** and checks the geometry and the format;
4. reads a **viewport** and compares it, byte for byte, with the canonical
   pixels converted for the display;
5. **picks** a pixel and compares it with the canonical pixel;
6. **fills** a 16×16 rectangle in the middle of the picture and reads it
   back;
7. **undoes** the fill and reads the rectangle back;
8. fills again and **saves**;
9. runs `vpng check` on the saved file;
10. compares the saved file's tile payloads with the original file's, tile
    by tile;
11. decodes the saved file and checks that the fill is there and that no
    other row moved.

### 2.4 The percentile rules

The repository's rules: the median is the middle sample, or the mean of the
two middle samples when the count is even; the p95 is the **nearest-rank**
percentile; the standard deviation is the sample one, with n − 1 in the
denominator.

### 2.5 The byte scope

The byte figures are **file bytes**: the size of the file the editor opened
and the size of the file it wrote. They do not include the editor's memory,
which is reported separately in §3.3.

### 2.6 The timing scope

The timings are the **wall clock of one HTTP round trip**, measured by the
Python client. They therefore include the client's own work and the
loopback stack. **They are not a performance claim about the editor** and
they are not compared against anything. They are recorded so that the
report can say what was measured.

### 2.7 The memory scope

The memory figure is the editor process's **peak working set**, taken from
the operating system's own `GetProcessMemoryInfo` while the process was
alive.

* **Included**: the operating system, the loader, the binary's text and
  data, the Rust heap, the decoded-tile cache, the display buffer, the undo
  history, and every other buffer the editor allocated.
* **Excluded**: nothing — the figure is the whole process.
* **Not a whole-picture operation**: the editor is **localized** — it reads
  and writes rectangles — so this figure *is* comparable with the project's
  64 MiB localized-editing target, unlike the whole-picture figures in the
  GPU and layer reports.

The editor's own buffers are reported separately, so the two can be told
apart.

---

## 3. What was measured

### 3.1 The headline

Every one of the 750 corpus images was opened in the editor, read, picked,
filled, undone, filled again and saved.

| Check | Result |
|---|---:|
| The viewport is byte-for-byte the canonical pixels | **750 / 750** |
| The pick is the pixel the file holds | **750 / 750** |
| The fill changes exactly the rectangle that was asked for | **750 / 750** |
| The undo puts the pixels back | **750 / 750** |
| The saved file holds the fill and nothing else | **750 / 750** |
| The saved file passes `vpng check` | **750 / 750** (0 failures) |
| Every untouched tile keeps its payload byte for byte | **750 / 750** |

**0 images failed.** The corpus pass took **774.319** seconds
wall-clock for 750 images, each with an encode, an editor process, eleven
HTTP round trips and a decode; that figure is a driver total, not a
benchmark.

Across the whole corpus the edit dirtied **989** tiles and the save copied
**7,955** — **8,944** tiles in all, which is every tile of every file. The
copied payloads carried **378,961,695** bytes and the rewritten ones
**74,338,188**. The two numbers are the engine's own accounting and the
measurement's independent one, and they agree on every file.

The viewer's planner ran on every picture: **2,378** tiles were visible and
**2,140** more were wanted next, and **2,140** of those were fetched.

### 3.2 A grey file takes the mean of the colour, rounded half up

The measurement filled with `(17, 34, 51, 255)`. A **grey** file cannot
hold a colour, so the editor takes the mean of the three colour channels,
rounded half up: `(17 + 34 + 51 + 1) / 3 = 34`. A grey file therefore
receives grey **34**, shown as `(34, 34, 34, 255)`.

The **first** pass of the measurement expected the colour itself and **49
grey images failed** — the checker's assumption was wrong, not the editor's
behaviour. The checker was corrected to ask what the file's own format can
hold, and the second pass is the one reported here. The failure is kept in
this report because it is the kind of thing a measurement is for: it proved
the rule is real rather than documented.

### 3.3 The memory figures, on the 16K photograph

The corpus's own 15,360 × 8,640 RGB photograph (398,131,200 canonical
bytes), shown through a 1024×1024 viewport, then panned, then edited over a
256×256 rectangle, then saved — five runs.

| Figure | Value |
|---|---:|
| The editor's peak working set (the largest of five runs) | **30,752,768 bytes** |
| The editor's peak working set (the median of five runs) | 30,732,288 bytes |
| The 64 MiB target | 67,108,864 bytes |
| The share of the target | **45.8%** |
| The decoded-tile cache's budget | 16,777,216 bytes |
| The undo history's budget | 16,777,216 bytes |
| The tiles in the file | 2,040 |
| The tiles in the viewport | 20 |
| The tiles the planner wanted next | 52 |

The five runs peaked at 30,466,048, 30,478,336, 30,732,288, 30,740,480
and 30,752,768 bytes — a spread of about 1% — and each run rewrote **2**
tiles and copied **2,038**.

The editor's peak is **below** the 64 MiB target, and it is below it while
holding the picture's **original level**: the editor reads the base level
through `Editor::read_region` and never allocates the whole picture.

The timings from the same run:

| Operation | Median |
|---|---:|
| The first viewport | 286.843 ms |
| A pan to a different 1024×1024 rectangle | 559.288 ms |
| A pick | 5.53 ms |
| A save | 215.463 ms |

These are the wall clock of one HTTP round trip, measured by the Python
client (§2.6). They are **not** a performance claim.

### 3.4 What the editor refuses

| The editor refuses | With | Because |
|---|---|---|
| A file with a **layer table** | `UnsupportedFeature`, a message that says "layer table" | the editing engine edits one layer of one grid and cannot rebuild the rest |
| A file with a **GPU payload table** | `UnsupportedFeature`, a message that says "GPU table" | the same reason |
| An **alpha edit** on a picture with no alpha channel | `UnsupportedFeature`, a message that says "alpha" | there is nothing for the tool to change, and saying so is better than doing nothing quietly |
| A **region larger than the limit** | `ResourceLimit` | the display buffer is four bytes per pixel and the source region beside it is one to eight; without a bound, a window showing a whole 16K picture would ask for half a gigabyte |
| A **rectangle outside the picture** | `InvalidArgument` | it is not a region of this picture |
| A request with **no session token** | HTTP 400 | the editor has no password and can write the file |
| A request whose **`Host` header** names another machine | HTTP 400 | the DNS-rebinding attack |
| A request with a **chunked body** | `UnsupportedFeature` | this protocol sends a `Content-Length` |

### 3.5 The test inventory

The editor adds **42 Rust tests** and **23 Node tests** of its own.

The Rust tests are three suites: the **session** (twelve tests — every
format, the exactness of a region, a pick, a fill, an undo, an alpha edit's
refusal and its effect, a 16-bit file staying 16-bit, the region limit, a
rectangle outside the picture, a save that keeps the untouched tiles, and a
saved file that can be opened again), the **protocol** (six tests — the
shape of a rectangle, a colour, the state and an error), and the **server**
(fourteen tests — the window, an unknown path, the token, the `Host`
header, the state, a region, a pick, a fill, a save, an undo and a redo, the
alpha refusal, the alpha edit, a tool, and the request counter). The
command line adds five more: `vpng ui` opens a file, it serves the picture
and saves an edit, it refuses a request without the token, it refuses a file
it cannot edit, and it needs a file.

The Node tests are the window's **geometry** — where a screen point is in
the picture, which rectangle a drag covers, how far the view may zoom out,
and the property that matters to the engine: **whatever the window asks for,
the engine can serve it**.

---

## 4. The twelve questions

**Q1 — Was the corpus actually run, or blocked?**
**Run, not blocked.** All **750** images were opened in the editor, read,
picked, filled, undone, filled again and saved. The raw file is
`benchmarks/p14_ui_results.json`.

**Q2 — How many times was each case run, and what do the percentiles mean?**
Each image is a **single run**, and the corpus is the repetition: 750 images
in one configuration. The 16K memory case is **five runs**, and its timings
are reported with a median. The percentile rules are the repository's: the
median is the middle sample, or the mean of the two middle samples when the
count is even; the p95 is the **nearest-rank** percentile; the standard
deviation is the sample one, with n − 1 in the denominator.

**Q3 — What is the baseline, and was it measured in the same run?**
**The corpus's canonical pixels, and the checker is independent of the
project's decoder.** The container is read from the specification — the
header's fields at their documented offsets, the directory entries at
theirs — and the display conversion is re-implemented from the ADR's rules.
The editor is compared against that, not against itself.

**Q4 — Does the editor change the pixels?**
**No, and that is measured.** The viewport the editor returns is byte-for-byte
the canonical pixels on **750 of 750** images. A fill changes exactly the
rectangle that was asked for on **750 of 750**, and an undo puts the pixels
back on **750 of 750**.

**Q5 — What does the editor buy?**
A window. It shows a picture, reads one pixel, changes a rectangle, undoes,
redoes and saves — over the engines the project already had, with no second
decoder and no second definition of the format.

**Q6 — How fast is it?**
**This milestone does not claim a speed.** §2.6 says why: the timings are
the wall clock of one HTTP round trip as measured by the Python client,
including the client's own work and the loopback stack. They are recorded,
not compared.

**Q7 — What does it cost?**
**30,752,768 bytes of peak working set** on the 16K photograph, against the
67,108,864-byte target — **45.8%**. The file's bytes do not move at all:
the editor writes the format the encoder wrote, and a save rewrites only the
tiles the edit dirtied.

**Q8 — Does the editor move the format's bytes or version?**
**No.** The editor introduces **no format revision**. It reads and writes
through the editing engine, so the header, the directory entries and the
tile payloads are exactly what they were. The default encoder's bytes did
not move either — the editor does not touch the encoder.

**Q9 — Is the format still binary-compatible?**
**Yes, and the milestone is not a format change at all.** There is nothing
to be compatible with: the editor writes the same files the command line
writes, through the same commit path.

**Q10 — Are the research codecs still out of reach?**
**Yes.** The editor changes nothing about the codec table. A directory entry
that names an unimplemented codec is still refused by the shared validation
the editor's reader uses, and the research ids `0x80..=0xFF` still cannot
reach a file.

**Q11 — What is in the memory figures?**
**The editor process's whole peak working set**, and §2.7 says exactly what
is in it: the operating system, the loader, the binary, the Rust heap, the
decoded-tile cache, the display buffer, the undo history and every other
buffer — nothing is excluded. The editor's own buffers are reported
separately, so the two can be told apart.

**Q12 — What is not measured, and what remains untested?**
The limits are in §6. The four that matter most: the editor does **not** use
the preview pyramid; it **refuses** a layered or a GPU file rather than
editing it; the measurement is a **program driven over HTTP**, not a person
driving a browser, so the pointer, the wheel and the keyboard are exercised
by the window's own geometry tests but not by a human; and **no other
platform** was measured — the editor is a local program and the measurement
host is Windows.

---

## 5. Reproducing the evidence

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# One: the editor's own tests.
cargo test -p vpng-ui
cargo test -p vpng-cli --test ui_command
node --test ui/test/geometry.test.mjs

# Two: the corpus.
cargo build --release -p vpng-cli
python benchmarks\ui\run_ui_corpus.py --jobs 1 --workers 4

# Three: the memory figures.
python benchmarks\ui\measure_ui_memory.py

# Four: the whole Rust suite, and the check that every quoted figure matches
# the raw data.
cargo test --workspace --all-targets
python benchmarks/verify_reports.py
```

---

## 6. What this does not claim

* **No claim that the editor is fast.** §2.6: the timings are the wall clock
  of one HTTP round trip as the Python client measured it, and they support
  no comparison.
* **No claim that the editor uses the preview pyramid.** It **does not**. The
  editor shows the picture's **original level**, even when the file carries
  derived preview levels, and the window says so in its banner. The reason
  is that a preview is a picture *of* the picture at a lower resolution, and
  editing it would mean editing a different image.
* **No claim that the editor can edit a layered or a GPU file.** It cannot:
  it **refuses** such a file with the editing engine's own reason. A refusal
  is not a capability.
* **No claim that a human was measured.** The corpus driver starts the real
  program and speaks its protocol; the pointer, the wheel and the keyboard
  are exercised by the window's geometry tests, not by a person.
* **No claim about other platforms.** The editor is a local program and the
  measurement host is Windows. The CI job runs the same suites on Linux.
* **No claim that the editor is a native application.** It is not. The window
  is the browser's canvas, and the editor is a local program that serves it.

---

## 7. The acceptance gates

| # | Gate | Result | Evidence |
|---|---|---|---|
| 1 | The corpus actually ran, at its full size | **PASS** | 750 of 750 images, eleven HTTP round trips each; `benchmarks/p14_ui_results.json` |
| 2 | The editor shows exactly what the file holds | **PASS** | 750 of 750 viewports are byte-for-byte the canonical pixels, checked against an independent reader |
| 3 | A pick is exact | **PASS** | 750 of 750 picks are the pixel the file holds |
| 4 | An edit changes exactly what was asked for | **PASS** | 750 of 750 fills, and 750 of 750 undos put the pixels back |
| 5 | A save rewrites only what changed | **PASS** | every untouched tile keeps its payload byte for byte on 750 of 750 images, and the rewritten and copied counts match the payloads |
| 6 | The saved file is still a valid container | **PASS** | 0 failures across the 750 saved files |
| 7 | The 16K case stays inside the memory target | **PASS** | **30,752,768 bytes** against the 67,108,864-byte target |
| 8 | The editor changes no byte of the format | **PASS** | no format revision, no new header field, no new directory flag |
| 9 | There is still one decoder | **PASS** | `vpng-view` plans and `vpng-editing` reads; the crate contains no codec |
| 10 | The editor is local-only and refuses an unauthenticated request | **PASS** | loopback bind, `Host` check, session token; 400 without the token |
| 11 | The refusals carry their reason | **PASS** | a layered file, a GPU file and an alpha edit on a file with no alpha are each refused with a message that names the cause |
| 12 | Every quoted figure is bound to the raw data | **PASS** | `check_ui()` in `benchmarks/verify_reports.py` |

---

## 8. What is next

The editor is in, measured and documented. What it leaves open:

* **The preview pyramid.** The editor shows the original level. A file with a
  pyramid could show its derived levels when zoomed out, and the editor
  would then need to distinguish "the level I am editing" from "the level I
  am showing".
* **The codec-mix panel.** The roadmap's v0.11 row asks the viewer to report
  the codec mix of the file it opened, so a user can see why the file is the
  size it is.
* **The dry-run comparison.** The roadmap's v0.13 row asks the editor to
  show what the file would weigh under a different codec before the commit,
  measured with the encoder's own selection function.
* **Editing a layered file.** The engine refuses one today; carrying the
  layer table through a commit is the next piece of work in the editing
  engine.
* **Then** the v0.2 → v0.23 release cadence, and **last** the v1.0 freeze.
