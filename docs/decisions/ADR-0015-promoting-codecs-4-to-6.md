# ADR-0015: Promoting the dictionary and context-adaptive codecs

**Status:** accepted
**Date:** 2026-11-22
**Related:** ADR-0004 (codec id table), ADR-0011 (predictor and entropy
coder), ADR-0014 (levels and partitions)
**Question:** The codec id table reserves ids 4, 5 and 6 for a dictionary
codec, a predictor over a dictionary, and context-adaptive entropy coding.
The research harness has measured candidates for all three. Which of them are
promoted to stable, under which ids, and does the default encoder use them?

## Decision

Promote three codecs to the stable set, under exactly the ids ADR-0004
reserved for them, and take the format's minor version to **3**.

| id | codec | what it is |
|---:|---|---|
| 4 | `lz77` | tile-local LZ77 dictionary over the tile's raw bytes |
| 5 | `predictor+dictionary` | the stable predictor's residuals, folded to unsigned bytes, then LZ77 |
| 6 | `context-entropy` | the stable predictor's residuals, priced against a gradient-conditioned probability table |

The research ids `0x80`, `0x81`, `0x83`, `0x92` and `0x93` stay
**research-only**: they may never reach a file, and the production decoder
rejects them with `UnsupportedFeature`.

Three rules follow from the promotion:

1. **A file that names id 4, 5 or 6 is a v0.3 file.** A v0.1 or v0.2 file that
   names one of them is malformed, and every reader rejects it with
   `UnsupportedFeature`.
2. **The minor version states what the file contains, not what the encoder was
   allowed to do.** A writer that permits the extended set but chooses only the
   original four codecs writes a v0.1 file. A writer that names a promoted
   codec must write minor version 3.
3. **The default codec set stays the original four.** The extended set is
   opt-in (`--codec-set extended`, or `EncodeOptions::with_extended_codecs`).

The selection rule is unchanged: the smallest payload wins, and a tie goes to
the **lowest codec id**. Raw is always a candidate.

## Rationale

* **The ids are already reserved.** ADR-0004 reserved 4, 5 and 6 for exactly
  these three pipelines. Using them means the reservation and the
  implementation agree, and no published id changes meaning.
* **The payloads did not change.** The three codecs were research code with
  research ids. The promotion moves them, re-identifies them and makes them
  reachable; the bitstreams are the ones the study measured, so the corpus
  numbers describe the code that ships.
* **The measurement justifies it.** Over the 750-image corpus the extended
  selection is **never larger** than the stable selection: 643 images are
  smaller, 107 are the same size, and **none** is larger. The total falls
  from 453,767,460 to 406,294,626 bytes, **−10.46%**, and the median ratio to
  the canonical pixels falls from 0.415337 to 0.357749. Every category gains:
  photos −6.30%, scans −18.01%, textures −13.57%, UI −13.23%, pixel art
  −15.10%. The full numbers, and the unfavourable ones, are in
  `docs/P8_CODEC_REPORT.md`.
* **The default does not move, and that is a deliberate choice.** The extended
  set costs **3.44×** the encode time of the stable set over the corpus with
  the production encoder: 421,618 ms against 122,542 ms for all 750 images
  (3.48× over the 643 images the two sets actually differ on). The research
  harness, which measures the candidates rather than the selector, put the
  same trade-off at 2.57× over its own subset. Turning the extended set on by
  default would trade a 10% size gain for a 3.4× encode-time cost, and would
  move every published byte and every golden fixture. The set stays opt-in
  until the cost is brought down; the *size* win is available today to anyone
  who asks for it.
* **The version rule is content-determined.** Tying the version to the
  options instead of the contents would force every extended encode — even one
  that chose only the original codecs — into a new version, and would move
  bytes that did not need to move. Tying it to the contents keeps the default
  path on v0.1 and makes the version a true statement about the file.
* **A v0.2 file cannot be silently reinterpreted.** The "minor version ≥ 3
  required" rule means an id 4/5/6 in an older file is a hard error rather
  than a best-effort decode. A file that claims a codec its version never
  defined is malformed, and saying so is the only safe reading.

## Consequences

* A v0.1 or v0.2 reader rejects a v0.3 file with `UnsupportedFeature` — the
  minor version is newer than the reader knows. A v0.3 reader accepts every
  v0.1 and v0.2 file.
* The **streaming writer** and the batch encoder must agree about the version,
  or the streaming writer would produce a file that names id 6 and claims to
  be v0.1. The rule now lives in one shared function
  (`vpng_encoder::content_minor`) that both call, and the test suite pins the
  agreement over 144 encode cases per codec set.
* The encoder writes **more bytes on a tile that no promoted codec helps**:
  the selection tries every permitted codec, so a tile that the old codecs
  already coded well now also pays for the attempts. The corpus shows the
  cost is bounded by the 2.57× encode-time figure above.
* An **edit** that rewrites a tile with a promoted codec bumps the committed
  file's minor version to 3. The untouched tiles keep their payload bytes
  exactly.
* The research harness keeps its own registry: `vpng-codecs`'s
  `experimental` module now holds only the research-only codecs, and the
  promoted ones live at the crate root.
