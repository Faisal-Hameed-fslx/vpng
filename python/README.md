# Python

Two things live here, and they are not the same thing.

## `vpng/` — the package

`vpng/` is the **Python binding to the VPNG engine**. It is a thin wrapper
around the Rust implementation: every pixel operation is the same code the
`vpng` command line runs, so the two can never disagree about what a file
means. It never implements a codec and never defines pixel semantics.

```python
import vpng

vpng.encode("input.png", "output.vpng")
assert vpng.verify("input.png", "output.vpng")

with vpng.open_document("output.vpng") as doc:
    region = doc.read_region(x=0, y=0, width=512, height=512)
    with doc.edit() as tx:
        tx.write_region(region)
        tx.commit()
```

Build it from the repository root:

```
maturin develop --release
```

The root `pyproject.toml` is the build definition; `maturin` is the backend.
The extension uses the **stable ABI** (`abi3-py38`), so one wheel works on
every CPython from 3.8 onwards.

The tests are in `tests/` and need nothing but the standard library — they
write and read their own PNG files, so there is no imaging dependency:

```
python -m unittest discover -s tests -v
```

## The tooling

The rest of the tree prepares the corpus, drives the Rust CLI, and renders
the reports. It **never** implements a codec or defines pixel semantics: the
Rust codec is the single source of truth.

* `experiments/` — throwaway notebooks and scripts for encoding-strategy ideas.
  Its own `pyproject.toml` holds the ruff configuration.
* `corpus/`     — corpus preparation and the images themselves (git-ignored).
* `tools/`      — reusable helpers, in particular a CLI runner.

The measurement drivers live one level up, in `benchmarks/`:
`benchmarks/python/` measures the binding against the command line.
