# Python tooling

Python here prepares the corpus, drives the Rust CLI, and renders reports. It
**never** implements a codec or defines pixel semantics: the Rust codec is the
single source of truth (roadmap §4, `vpng-python` must wrap the stable Rust APIs).

Layout:

* `experiments/` — throwaway notebooks and scripts for encoding-strategy ideas.
* `corpus/`     — corpus preparation and the images themselves (git-ignored).
* `tools/`      — reusable helpers, in particular a CLI runner.
