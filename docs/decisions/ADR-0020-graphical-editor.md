# ADR-0020: the graphical editor's shape

**Status:** accepted
**Date:** 2026-11-28
**Related:** ADR-0012 (atomic replacement), ADR-0013 (cache eviction),
ADR-0014 (levels and partitions), ADR-0017 (the WebAssembly package's
shape and ABI), ADR-0019 (the GPU payload), the roadmap §10.4 and §21.1
(the graphical editor, P11)
**Question:** The roadmap asks for a "graphical editor" as P11. The
question is: what is the editor made of, and — the question that decides
everything else — **is it a native window or a browser?**

## Decision

### 1. The editor is a local program with a browser front end

The editor is a Rust program that:

* opens one VPNG file through the **editing engine**;
* serves a small web application on the **loopback** interface;
* opens the browser at that address.

The window is the browser's canvas. The editor is a **view and a set of
commands**; the editing engine does the decoding, the changing and the
writing.

The alternative — a native toolkit such as egui or Slint — was rejected
for three reasons, each of which is a property of this repository rather
than a matter of taste:

* **The engines are already here.** `vpng-view` decides which tiles are on
  screen, and `vpng-editing` decodes them, changes them and commits them.
  A native window would add a windowing stack, an event loop and a drawing
  library to a workspace that deliberately has none — and the drawing code
  would be new code with no other purpose.
* **The workspace's minimum Rust version is 1.74.** The current graphical
  toolkits require a newer compiler. Raising the minimum version for a
  window would be a cost paid by every consumer of the library for the
  benefit of one program.
* **A browser window can be tested.** The editor's whole surface is HTTP
  and JSON, so "open a file, read a rectangle, fill a rectangle, save" is
  exercised by the ordinary test suite and by the continuous-integration
  job. A native window needs a display server, and the continuous-
  integration job has none.

The HTTP server is **hand-written** — a few hundred lines over
`std::net::TcpListener` — for the same reason: no asynchronous runtime and
no web framework enter the workspace. The server speaks HTTP/1.1 only as
far as a browser's `fetch` needs it, and every response closes its
connection.

### 2. The editor is a client of the format; it does not change it

The editor reads and writes files through the existing engines. It
introduces **no format revision**: the header, the directory, the tile
payloads and the layer table are exactly what they were. The editor is a
consumer of the format, not a second definition of it.

Every edit goes through `Editor::write_region` and every save through
`Editor::commit`, so the guarantees the editing engine already has are the
editor's guarantees:

* a save rewrites **only** the tiles the edit dirtied, and copies every
  other tile's payload byte for byte;
* the commit writes a temporary file and renames it over the target, so a
  failure leaves the original untouched;
* an undo restores the previous pixels exactly, because the history holds
  the pixels rather than a picture of them.

### 3. The editor shows the original level, and says so

The editor reads the **base level** through `Editor::read_region`. It does
**not** use the preview pyramid: a v0.2 file's derived levels are not
displayed, and the window says so in its banner.

The reason is that the editor is an *editor*, and a preview is a picture
of the picture at a lower resolution — editing it would mean editing a
different image. The pyramid is a **viewing** device, and the viewing
device here is the base level plus a limit on how many pixels one request
may carry.

That limit is the editor's memory bound: `DEFAULT_MAX_REGION_PIXELS` is
4,194,304 — a 2048×2048 rectangle. The window clamps its zoom-out to fit
the limit, and the engine refuses a request that exceeds it.

### 4. The viewer engine plans; the editing engine reads

The editor uses `vpng-view` for what it is good at: deciding which tiles
are on screen and which are wanted next. `Viewport::plan` yields the
visible tiles and the prefetch ring.

The prefetch is carried out by the **editing engine**, one tile at a
time, so there is still **one decoder** in the process and the transient
buffer is one tile rather than the whole ring.

### 5. The display is 8-bit; the file is not

A browser's canvas takes RGBA8. A file may hold 16-bit samples, a grey
channel, or no alpha channel at all. The editor converts the file's pixels
to RGBA8 **for the display only**; an edit is written back in the file's
own format.

The conversion rules are fixed:

| file format | display |
|-------------|---------|
| `gray8`, `gray16` | the grey sample in all three colour channels, opaque |
| `graya8`, `graya16` | the grey sample in the three colour channels, the alpha as it is |
| `rgb8`, `rgb16` | the three colour channels, opaque |
| `rgba8`, `rgba16` | the four channels as they are |

A 16-bit sample is scaled down by its top byte (`value >> 8`) for the
display.

The reverse direction — the colour the user chose, written into the file —
has equally fixed rules:

* a **grey** file takes the mean of the colour's three channels, rounded
  half up: `(r + g + b + 1) / 3`;
* a **16-bit** file takes the 8-bit value scaled by 257, so 255 becomes
  65535 exactly;
* a file with **no alpha channel** ignores the colour's alpha;
* the **alpha tool** is refused on a file with no alpha channel, with the
  reason, rather than doing nothing quietly.

### 6. The editor is local-only, and says so

The editor has **no password** and it can write the file it opens. It is
therefore bound to the loopback interface and to nothing else, and it
checks two things before it answers a request that could change anything:

* the **`Host` header** must name a loopback address, so a web page cannot
  reach the editor through a name that resolves to the machine — the
  DNS-rebinding attack;
* the **session token** must be presented in the `X-Vpng-Token` header. The
  token comes from the operating system's random source and is printed in
  the address the editor opens.

The static assets — the page, the script, the stylesheet, the geometry
module — are served without the token, because they are text and carry no
authority.

### 7. The front end is source, not build output

The four files of the window live in the repository's `ui/` directory,
beside the WebAssembly package's `wasm/` directory, and are **compiled into
the binary** with `include_str!`. The editor is one program the user runs;
it does not need a folder of files beside it, and it cannot be pointed at
a different front end by accident.

The arithmetic — where a screen point is in the picture, which rectangle a
drag covers, how far the view may zoom out — is in `ui/geometry.js`, which
is **pure**: it touches no DOM, no canvas and no network. It is tested on
its own in Node, with no browser and no display server.

## Rationale

The alternative designs each cost something this repository cannot pay:

* a **native toolkit** costs the minimum Rust version, a windowing
  dependency, and the ability to test the editor at all;
* a **JavaScript image decoder** would be a second implementation of the
  format — the thing this repository exists not to have — and it would
  mean the browser and the engine could disagree about a picture;
* a **desktop application bridge** (a local socket, a subprocess) is the
  browser front end with more moving parts.

The browser front end has one cost of its own: the picture crosses the
network stack, even when that stack is the loopback. The measurement in
`docs/P14_UI_REPORT.md` quantifies it rather than assuming it.

## Consequences

* The workspace gains one crate, `vpng-ui`, and one directory, `ui/`. It
  gains **no new dependencies** beyond what the HTTP client in
  `vpng-http` already uses, and that one is a **test** dependency here.
* The editor is a **client** of the format: no format revision, no new
  header field, no new directory flag.
* The editor **refuses** a file that carries a layer table or a GPU
  payload table, because the editing engine edits one layer of one grid
  and cannot rebuild the rest. The refusal carries the reason.
* The editor does not use the preview pyramid. A future build may; the
  milestone report says plainly that this one does not.
* The editor can be driven by anything that speaks HTTP, which is what the
  measurement driver does and what the test suite does.
