# ADR-0012: Atomic replacement and the temporary-file rewrite

**Status:** accepted
**Date:** 2026-11-16
**Question (roadmap §18.9, §18.10):** How is an edited file replaced
atomically on Windows, Linux and macOS, and is the format append-only or a
temporary-file rewrite?

## Decision

An editing session writes a **new file to a temporary path in the same
directory as the target**, flushes it, `sync_all`s it, and then replaces the
target with a single `std::fs::rename`. The original file is never opened
for writing.

| Platform | The replacement |
|---|---|
| Linux | `rename(2)` over the target — atomic within a filesystem |
| macOS | `rename(2)` over the target — atomic within a filesystem |
| Windows | `std::fs::rename`, which is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` — a one-step replace |

The temporary file is a sibling of the target, so the replacement is always
a rename **within one volume** and can never cross a filesystem boundary,
where a rename would degrade to a copy.

The format is **not** append-only. An edit produces a new file: a new
header, a new directory, the dirty tiles re-encoded, and every untouched
tile's payload copied **byte for byte**. Append-only generations remain the
*fallback* for a platform on which an atomic replace is unavailable; in that
case the new generation is not committed until a directory generation that
references it has been written and its checksum has validated.

## Rationale

* **The original must survive every failure.** A writer that modifies the
  original in place has, by construction, a window in which the file on
  disk is neither the old state nor the new one. A temporary-file rewrite
  has no such window: the target is either the old file or the new one.
* **A rename is the only portable atomic replace.** POSIX gives `rename(2)`
  the atomicity guarantee; Windows gives `MoveFileExW` with
  `MOVEFILE_REPLACE_EXISTING`. Both are one syscall, and both are atomic
  with respect to a concurrent reader opening the path.
* **A sibling temporary file is required, not merely convenient.** A
  temporary file on a different volume would make the final step a
  non-atomic copy.
* **Byte-level payload reuse is what makes the rewrite affordable.** The
  new file is not a re-encode of the whole image: every untouched tile's
  payload is copied verbatim, so a one-tile edit rewrites one tile and
  copies the rest. The payload's *offset* may move, because the container
  lays payloads out contiguously; the bytes do not change.
* **Append-only is a good fallback and a poor default.** It needs a second
  index and a reclamation policy, and it grows the file without bound. It
  is kept for platforms that cannot replace a file atomically, not chosen
  where the guarantee is available.

## Consequences

* The save path is in `crates/vpng-editing/src/commit.rs`. It writes
  `.{name}.{pid}.{counter}.vpng-edit.tmp`, flushes, `sync_all`s, renames,
  and removes the temporary file on any failure it observes.
* **Failure is tested, not asserted.** `CommitOptions::failure` injects a
  failure at `FailurePoint::AfterBytes(n)` (the write fails part-way) and at
  `FailurePoint::BeforeRename` (the temporary file is complete but the
  replacement is refused). In both cases the original is byte-identical to
  what it was, it still parses, it still passes its integrity check, and no
  temporary file is left behind.
* **The format does not change.** An edited file is an ordinary VPNG file
  with the same 64-byte header, the same 40-byte directory entries and the
  same payloads. The editor carries the original header's fields over
  unchanged and recomputes only the two fields that are defined in terms of
  the content: the directory offset and the declared file size.
* **A power cut between the `sync_all` and the rename leaves the original
  intact and a temporary file behind.** The engine removes its own
  temporary file when it observes a failure; it does not sweep the
  directory for a temporary file left by a killed process. Sweeping is a
  later task.
* **There is no file locking.** Two editors over the same file do not see
  each other's changes, and the second save wins. Locking is a later task.
