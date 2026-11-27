"""VPNG — a lossless, adaptive, tiled, random-access image format.

This package is a thin wrapper around the Rust implementation. Every pixel
operation is the same code the ``vpng`` command line runs, so the two can
never disagree about what a file means.

The rules this library keeps:

* **Lossless, always.** There is no lossy mode and no lossy fallback. A
  request for one is refused, not quietly ignored.
* **Exact.** :func:`verify` compares the decoded pixels with the original,
  byte for byte.
* **Deterministic.** The same pixels and the same options produce the same
  file, whatever the worker count.
* **Never a partial file.** An encode writes to a temporary file and renames
  it over the target, so a reader never sees half an image.

A short tour::

    import vpng

    vpng.encode("input.png", "output.vpng")
    assert vpng.verify("input.png", "output.vpng")

    with vpng.open_document("output.vpng") as doc:
        region = doc.read_region(x=0, y=0, width=512, height=512)
        with doc.edit() as tx:
            tx.write_region(region)
            tx.commit()

The pixel bytes Python sees are **canonical**: rows are tightly packed with
no padding, and a 16-bit sample is little-endian. That is the same order the
format stores, so a region can be handed straight to ``write_region``.
"""

from __future__ import annotations

import os
from typing import Any

from . import _vpng
from ._vpng import (
    CheckResult,
    CommitResult,
    DecodeResult,
    Document,
    EncodeResult,
    Region,
    Transaction,
)

__all__ = [
    "VpngError",
    "InvalidArgumentError",
    "UnsupportedFeatureError",
    "CorruptFileError",
    "ResourceLimitError",
    "SizeLimitExceededError",
    "VerificationFailedError",
    "VpngIOError",
    "InternalError",
    "Region",
    "EncodeResult",
    "DecodeResult",
    "CheckResult",
    "CommitResult",
    "Document",
    "Transaction",
    "encode",
    "encode_pixels",
    "encode_layers",
    "decode",
    "decode_to_bytes",
    "decode_region",
    "layers",
    "decode_layer",
    "decode_layer_to_bytes",
    "composite",
    "composite_to_bytes",
    "verify",
    "check",
    "info",
    "open_document",
    "pixel_format",
    "FORMAT_VERSION",
    "DEFAULT_TILE_SIZE",
    "DEFAULT_CACHE_BYTES",
    "FORMATS",
    "CODECS",
    "CODEC_SETS",
    "__version__",
]

#: The base class of every error VPNG raises.
#:
#: The Rust extension owns this type so that a failure raised before the
#: Python wrapper finished importing is still a ``VpngError``.
VpngError = _vpng.VpngError


class InvalidArgumentError(VpngError, ValueError):
    """The caller passed an argument that is syntactically or semantically wrong."""


class UnsupportedFeatureError(VpngError, NotImplementedError):
    """The file uses a feature, version, or codec this build does not implement."""


class CorruptFileError(VpngError, ValueError):
    """The bytes are inconsistent with the VPNG specification."""


class ResourceLimitError(VpngError, MemoryError):
    """A configured resource budget would be exceeded."""


class SizeLimitExceededError(VpngError, ValueError):
    """No lossless representation fits inside the requested size ceiling."""


class VerificationFailedError(VpngError, ValueError):
    """Decoding succeeded but the result did not match the expected pixels."""


class VpngIOError(VpngError, OSError):
    """The operating system refused an I/O operation."""


class InternalError(VpngError, RuntimeError):
    """A bug: an invariant the implementation believed in did not hold."""


# The extension looks the classes up by the stable error label, so a failure
# carries the class that matches its kind. The labels are the same ones the
# Rust `ErrorKind::label` returns and never change.
_vpng._register_exceptions(
    {
        "invalid-argument": InvalidArgumentError,
        "unsupported-feature": UnsupportedFeatureError,
        "corrupt-file": CorruptFileError,
        "resource-limit": ResourceLimitError,
        "size-limit-exceeded": SizeLimitExceededError,
        "verification-failed": VerificationFailedError,
        "io": VpngIOError,
        "internal": InternalError,
    }
)

#: The version of the VPNG container this extension writes.
FORMAT_VERSION = _vpng.FORMAT_VERSION

#: The default tile edge, in pixels.
DEFAULT_TILE_SIZE = _vpng.DEFAULT_TILE_SIZE

#: The default decoded-tile cache budget, in bytes.
DEFAULT_CACHE_BYTES = _vpng.DEFAULT_CACHE_BYTES

#: The canonical pixel formats, as the Python side spells them.
FORMATS = tuple(_vpng.FORMATS)

#: The stable codec ids, in ascending order.
CODECS = tuple(_vpng.CODECS)

#: The codec sets a file may be encoded with.
CODEC_SETS = tuple(_vpng.CODEC_SETS)

#: The version of this package.
__version__ = _vpng.__version__


def encode(
    input: str | os.PathLike[str],
    output: str | os.PathLike[str],
    *,
    lossless: bool = True,
    max_size: int | None = None,
    adaptive_tiles: bool = False,
    pyramid_levels: int = 0,
    codec: str = "auto",
    codec_set: str = "stable",
    tile_size: int = DEFAULT_TILE_SIZE,
    workers: int = 1,
) -> EncodeResult:
    """Encode a picture into a ``.vpng`` file.

    The file is written through a temporary file in the target's directory and
    renamed over it, so a failed encode leaves no partial file behind.

    ``lossless`` must be true: VPNG has no lossy mode, and there is never a
    lossy fallback.

    ``codec_set`` is ``"stable"`` (the four original codecs) or ``"extended"``
    (which adds the three codecs promoted in format v0.3). The worker count
    changes the speed, never the bytes.
    """
    return _vpng.encode(
        os.fspath(input),
        os.fspath(output),
        lossless=lossless,
        max_size=max_size,
        adaptive_tiles=adaptive_tiles,
        pyramid_levels=pyramid_levels,
        codec=codec,
        codec_set=codec_set,
        tile_size=tile_size,
        workers=workers,
    )


def encode_pixels(
    pixels: bytes,
    width: int,
    height: int,
    output: str | os.PathLike[str],
    *,
    format: str = "rgba8",
    lossless: bool = True,
    max_size: int | None = None,
    adaptive_tiles: bool = False,
    pyramid_levels: int = 0,
    codec: str = "auto",
    codec_set: str = "stable",
    tile_size: int = DEFAULT_TILE_SIZE,
    workers: int = 1,
) -> EncodeResult:
    """Encode canonical pixel bytes straight into a ``.vpng`` file.

    The pixels are **canonical**: rows are tightly packed with no padding, and
    a 16-bit sample is little-endian. This is the same order the format
    stores, and the same order :func:`decode_to_bytes` returns.

    This is the entry point for a caller that already has the pixels in
    memory; it never writes an intermediate PNG.
    """
    return _vpng.encode_pixels(
        bytes(pixels),
        width,
        height,
        os.fspath(output),
        format=format,
        lossless=lossless,
        max_size=max_size,
        adaptive_tiles=adaptive_tiles,
        pyramid_levels=pyramid_levels,
        codec=codec,
        codec_set=codec_set,
        tile_size=tile_size,
        workers=workers,
    )


def encode_layers(
    output: str | os.PathLike[str],
    layers: list[dict[str, Any]],
    *,
    format: str = "rgba8",
    tile_size: int = DEFAULT_TILE_SIZE,
    workers: int = 1,
) -> EncodeResult:
    """Encode several layers into a ``.vpng`` file.

    The file is a **v0.4** file: it carries a layer table, and its directory
    entries name the layer each tile belongs to. The layers are encoded
    independently of one another, and each layer's bytes are accounted for on
    its own.

    Each layer is a mapping with these keys:

    * ``pixels`` — the layer's canonical pixel bytes (**required**);
    * ``width`` and ``height`` — the shape of those pixels (**required**);
    * ``name`` — the layer's name (default: empty);
    * ``kind`` — ``"pixels"`` (the default) or ``"mask"``;
    * ``x`` and ``y`` — where the layer sits on the canvas (default: 0, 0);
    * ``opacity`` — 0 to 255 (default: 255);
    * ``mask`` — the index of the layer that masks this one (default: none).

    The base layer is layer 0: it must cover the whole canvas::

        vpng.encode_layers(
            "layered.vpng",
            [
                {"name": "base", "pixels": base_pixels, "width": 64, "height": 48},
                {"name": "ink", "pixels": ink_pixels, "width": 32, "height": 16,
                 "x": 8, "y": 8, "mask": 2},
                {"name": "m", "kind": "mask", "pixels": mask_pixels,
                 "width": 32, "height": 16, "x": 8, "y": 8},
            ],
        )
    """
    return _vpng.encode_layers(
        os.fspath(output),
        layers,
        format=format,
        tile_size=tile_size,
        workers=workers,
    )


def decode(
    source: str | os.PathLike[str],
    output: str | os.PathLike[str],
    *,
    level: int = 0,
) -> DecodeResult:
    """Decode a ``.vpng`` file into a picture.

    ``level`` picks a preview level; zero (the default) is the original, which
    is always exact. The output is a PNG, or a raw file when the path ends in
    ``.raw``.
    """
    return _vpng.decode(os.fspath(source), os.fspath(output), level=level)


def decode_to_bytes(source: str | os.PathLike[str], *, level: int = 0) -> bytes:
    """Decode a ``.vpng`` file into canonical pixel bytes.

    The bytes are **canonical**: rows are tightly packed with no padding, and
    a 16-bit sample is little-endian. This is the same order the format
    stores.
    """
    return _vpng.decode_to_bytes(os.fspath(source), level=level)


def decode_region(
    source: str | os.PathLike[str],
    output: str | os.PathLike[str],
    x: int,
    y: int,
    width: int,
    height: int,
) -> DecodeResult:
    """Decode a region of a ``.vpng`` file into a picture."""
    return _vpng.decode_region(os.fspath(source), os.fspath(output), x, y, width, height)


def layers(source: str | os.PathLike[str]) -> list[dict[str, Any]]:
    """List the layers of a ``.vpng`` file.

    A file written before format v0.4 has exactly one layer: the base layer,
    which is the picture itself. A v0.4 file may hold more, each with a name,
    a rectangle on the canvas, an opacity and — optionally — a mask.

    Every layer is reported with its own byte accounting: the pixels it holds
    and the bytes its payloads take.
    """
    return _vpng.layers(os.fspath(source))


def decode_layer(
    source: str | os.PathLike[str],
    output: str | os.PathLike[str],
    layer: int,
) -> DecodeResult:
    """Decode one layer's own pixels into a picture.

    The result is sized to the **layer's own rectangle**, not to the canvas,
    and it holds the layer's pixels exactly. A mask layer decodes to its
    coverage image: the same pixels, of which only the alpha channel carries
    meaning.
    """
    return _vpng.decode_layer(os.fspath(source), os.fspath(output), layer)


def decode_layer_to_bytes(source: str | os.PathLike[str], layer: int) -> bytes:
    """Decode one layer's own pixels into canonical pixel bytes.

    The bytes are **canonical**: rows are tightly packed with no padding, and
    a 16-bit sample is little-endian.
    """
    return _vpng.decode_layer_to_bytes(os.fspath(source), layer)


def composite(source: str | os.PathLike[str], output: str | os.PathLike[str]) -> DecodeResult:
    """Composite every layer into one picture.

    The base layer is the canvas, exactly. Each layer above it is blended over
    the result with its own opacity and — when it has one — its mask. A file
    with one layer composites to exactly that layer's pixels.
    """
    return _vpng.composite(os.fspath(source), os.fspath(output))


def composite_to_bytes(source: str | os.PathLike[str]) -> bytes:
    """Composite every layer into canonical pixel bytes."""
    return _vpng.composite_to_bytes(os.fspath(source))


def verify(input: str | os.PathLike[str], source: str | os.PathLike[str]) -> bool:
    """Compare a ``.vpng`` file with the picture it was made from.

    Returns ``True`` when the decoded pixels are **exactly** the original's,
    and ``False`` when they are not. A file that cannot be read or decoded
    raises instead: that is a different failure from "the pixels differ".
    """
    return _vpng.verify(os.fspath(input), os.fspath(source))


def check(source: str | os.PathLike[str]) -> CheckResult:
    """Check a ``.vpng`` file's internal integrity.

    Every tile payload's CRC32C is verified. No pixels are decoded and no
    pixel buffer is allocated.
    """
    return _vpng.check(os.fspath(source))


def info(source: str | os.PathLike[str]) -> dict[str, Any]:
    """Read a ``.vpng`` file's header and directory.

    This is O(1) in the tile count, and it cannot detect a corrupted payload:
    use :func:`check` for that.
    """
    return _vpng.info(os.fspath(source))


def open_document(
    source: str | os.PathLike[str],
    *,
    cache_bytes: int = DEFAULT_CACHE_BYTES,
    history_bytes: int | None = None,
) -> Document:
    """Open a ``.vpng`` file for editing.

    The document is a context manager. ``cache_bytes`` bounds the
    decoded-tile cache; ``history_bytes`` bounds the undo history.
    """
    if history_bytes is None:
        return _vpng.open_document(os.fspath(source), cache_bytes=cache_bytes)
    return _vpng.open_document(
        os.fspath(source), cache_bytes=cache_bytes, history_bytes=history_bytes
    )


def pixel_format(name: str) -> str:
    """The canonical name of a pixel format such as ``rgba8`` or ``rgb16``."""
    return _vpng.pixel_format(name)
