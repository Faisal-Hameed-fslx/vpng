# The VPNG graphical editor

`vpng ui <file.vpng>` opens the file in a graphical editor. The editor is a
local program: it serves a small web application on the loopback interface
and opens it in your browser.

```console
$ vpng ui picture.vpng
vpng: editing picture.vpng
vpng: the editor is at http://127.0.0.1:53219/?token=9f3c...
vpng: press Ctrl+C to stop it
```

## What it does

| tool | what it does |
|------|--------------|
| **Pan** (`V`) | move the view; drag with the mouse or the middle button |
| **Pick** (`I`) | read the pixel under the pointer, and adopt its colour |
| **Fill** (`F`) | fill a dragged rectangle with the current colour |
| **Alpha** (`A`) | set the alpha channel over a dragged rectangle |

`Ctrl+Z` undoes, `Ctrl+Y` (or `Ctrl+Shift+Z`) redoes, and `Ctrl+S` saves.
The **Fit** button shows the whole picture and **1:1** shows it at its own
size. The status bar carries the picture's size, its format, the pointer's
position, the pixel under the pointer, the tile counts and the editor's own
cache and history sizes.

## What it is exact about

* **The picture it shows is the picture the file holds.** A viewport comes
  back as RGBA8 and is byte-for-byte the file's own pixels, converted for
  the display.
* **An edit changes exactly the rectangle you asked for.** Nothing else
  moves.
* **An undo puts the pixels back.**
* **A save rewrites only what changed.** Every tile the edit did not touch
  keeps its payload byte for byte, and the saved file still passes
  `vpng check`.

## What it refuses

* **A file with a layer table.** The editing engine edits one layer of one
  grid and cannot rebuild the rest, so the editor says so rather than
  editing something it cannot finish.
* **A file with a GPU payload table.** The same reason.
* **An alpha edit on a picture with no alpha channel.** There is nothing
  for the tool to change, and the editor says that.
* **A region larger than the limit.** The editor's limit is 4,194,304
  pixels — a 2048×2048 rectangle. The window clamps its zoom-out to fit it.

## What it does not do

* **It does not use the preview pyramid.** The editor shows the picture's
  original level, even when the file carries derived preview levels. The
  window says so in its banner.
* **It does not draw with a GPU.** The GPU payload is a separate,
  explicitly named, lossy feature; the editor never reads it.
* **It is not on the network.** The editor listens on the loopback
  interface only, and it checks that the `Host` header names a loopback
  address and that the session token is presented.

## The options

```
--port <n>        The port to listen on. 0 asks the operating system for a
                  free one. The default is 0.
--max-region <n>  The largest number of pixels one display request may
                  carry. The default is 4194304.
--no-browser      Do not open a browser; print the address instead.
```

## The files

| file | what it is |
|------|------------|
| `ui/index.html` | the page |
| `ui/style.css` | the stylesheet |
| `ui/app.js` | the wiring: the pointer, the keyboard, the canvas |
| `ui/geometry.js` | the arithmetic: where a screen point is in the picture |
| `ui/test/geometry.test.mjs` | the geometry's test suite |

The four front-end files are **compiled into the binary** with
`include_str!`, so the editor is one program and needs no folder of files
beside it. They live in the repository as source, beside the WebAssembly
package's `wasm/` directory.

## Testing it

```console
$ node --test ui/test/geometry.test.mjs
$ cargo test -p vpng-ui
$ cargo test -p vpng-cli --test ui_command
```

The geometry module is pure — no DOM, no canvas, no network — so it is
tested in Node without a browser. The editor itself needs no display
server, so its whole surface is exercised by the Rust test suite and by
the continuous-integration job: the program is started, its address is
used, and the picture that comes back is checked.

## The measurement

```console
$ python benchmarks/ui/run_ui_corpus.py --jobs 1
$ python benchmarks/ui/measure_ui_memory.py
```

The first drives the real `vpng ui` program over the 750-image corpus and
checks every picture; the second measures the editor's peak working set on
the 16K photograph. The figures are in `docs/P14_UI_REPORT.md`.

Measured over the whole 750-image corpus: the viewport is byte-for-byte the
canonical pixels on **750 of 750** images, the pick is exact on **750 of
750**, the fill changes exactly the rectangle asked for on **750 of 750**,
the undo puts the pixels back on **750 of 750**, the saved file holds the
fill and nothing else on **750 of 750**, **0** saved files failed
`vpng check`, and every untouched tile kept its payload byte for byte. On
the 16K photograph the editor's peak working set is **30,752,768 bytes**
against the 67,108,864-byte target — **45.8%**.
