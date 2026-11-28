"""A small HTTP client and a small VPNG container reader.

The client speaks the editor's protocol: JSON in, JSON out, with the
picture's pixels coming back as raw bytes. It is deliberately written on
the standard library alone, so the measurement needs nothing installed
beyond Python itself.

The container reader is written from the **specification**, not from the
project's own code: the header is read field by field and the directory is
read entry by entry. That matters for a measurement — if the reader shared
the decoder's code, a bug in the decoder would hide itself. Here the
reader is an independent witness, and it can disagree with the file.
"""

from __future__ import annotations

import json
import struct
import urllib.error
import urllib.request
from pathlib import Path

#: The header's magic bytes.
MAGIC = bytes([0x56, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])

#: The size of one directory entry, in bytes.
ENTRY_SIZE = 40


class EditorError(RuntimeError):
    """The editor answered with an error."""

    def __init__(self, status: int, body: bytes):
        self.status = status
        self.body = body
        message = body.decode("utf-8", "replace")
        try:
            payload = json.loads(message)
            message = payload["error"]["message"]
        except (json.JSONDecodeError, KeyError, TypeError):
            pass
        super().__init__(f"the editor answered {status}: {message}")


class Editor:
    """A running editor, driven over HTTP."""

    def __init__(self, base: str, token: str):
        self.base = base.rstrip("/")
        self.token = token

    def request(self, path: str, body: dict | None = None) -> tuple[int, bytes, float]:
        """Send one request and return its status, its body and its time."""
        import time

        url = f"{self.base}{path}"
        data = None
        headers = {"X-Vpng-Token": self.token}
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"

        started = time.perf_counter()
        request = urllib.request.Request(url, data=data, headers=headers)
        try:
            with urllib.request.urlopen(request) as response:
                payload = response.read()
                status = response.status
        except urllib.error.HTTPError as error:
            payload = error.read()
            status = error.code
        elapsed = (time.perf_counter() - started) * 1000
        if status >= 400:
            raise EditorError(status, payload)
        return status, payload, elapsed

    def json(self, path: str, body: dict | None = None) -> tuple[dict, float]:
        """Send one request and read its JSON answer."""
        _, payload, elapsed = self.request(path, body)
        return json.loads(payload.decode("utf-8")), elapsed

    def state(self) -> dict:
        """The editor's state."""
        payload, _ = self.json("/api/state")
        return payload

    def region(self, x: int, y: int, width: int, height: int) -> tuple[bytes, float]:
        """Read a rectangle of the picture, as RGBA8."""
        _, payload, elapsed = self.request(
            "/api/region",
            {"x": x, "y": y, "width": width, "height": height},
        )
        return payload, elapsed

    def pick(self, x: int, y: int) -> tuple[dict, float]:
        """Read one pixel."""
        payload, elapsed = self.json("/api/pick", {"x": x, "y": y})
        return payload, elapsed

    def colour(self, red: int, green: int, blue: int, alpha: int) -> None:
        """Choose the current colour."""
        self.json(
            "/api/colour",
            {"r": red, "g": green, "b": blue, "a": alpha},
        )

    def fill(self, x: int, y: int, width: int, height: int) -> tuple[dict, float]:
        """Fill a rectangle."""
        payload, elapsed = self.json(
            "/api/fill", {"x": x, "y": y, "width": width, "height": height}
        )
        return payload, elapsed

    def set_alpha(self, x: int, y: int, width: int, height: int, value: int) -> dict:
        """Set the alpha channel over a rectangle."""
        payload, _ = self.json(
            "/api/alpha",
            {"x": x, "y": y, "width": width, "height": height, "value": value},
        )
        return payload

    def undo(self) -> dict:
        """Undo the newest edit."""
        payload, _ = self.json("/api/undo", {})
        return payload

    def redo(self) -> dict:
        """Redo the newest undone edit."""
        payload, _ = self.json("/api/redo", {})
        return payload

    def save(self, path: str | None = None) -> tuple[dict, float]:
        """Save the document."""
        body = {} if path is None else {"path": path}
        payload, elapsed = self.json("/api/save", body)
        return payload, elapsed


class Tile:
    """One entry of a tile directory."""

    __slots__ = (
        "x",
        "y",
        "width",
        "height",
        "codec_id",
        "payload_offset",
        "compressed_length",
        "payload_checksum",
    )

    def __init__(self, **fields):
        for name, value in fields.items():
            setattr(self, name, value)


class Container:
    """A VPNG container, read from its bytes."""

    def __init__(self, data: bytes):
        self.data = data
        if data[:8] != MAGIC:
            raise ValueError("the file does not start with the VPNG magic")
        (
            self.major,
            self.minor,
            self.header_size,
            self.width,
            self.height,
            self.channels,
            self.bits,
            self.color_model,
            self.default_codec,
            self.tile_count,
        ) = struct.unpack_from("<HHIIIBBBBI", data, 8)
        (
            self.directory_offset,
            self.declared_file_size,
        ) = struct.unpack_from("<QQ", data, 32)
        (self.feature_flags,) = struct.unpack_from("<I", data, 48)

        self.tiles: list[Tile] = []
        for index in range(self.tile_count):
            start = self.directory_offset + index * ENTRY_SIZE
            (
                x,
                y,
                width,
                height,
                codec_id,
                predictor_id,
                flags,
                payload_offset,
                compressed_length,
                uncompressed_length,
                payload_checksum,
            ) = struct.unpack_from("<IIHHBBHQQII", data, start)
            self.tiles.append(
                Tile(
                    x=x,
                    y=y,
                    width=width,
                    height=height,
                    codec_id=codec_id,
                    payload_offset=payload_offset,
                    compressed_length=compressed_length,
                    payload_checksum=payload_checksum,
                )
            )

    def payload(self, tile: Tile) -> bytes:
        """One tile's payload bytes, exactly as the file stores them."""
        start = tile.payload_offset
        return self.data[start : start + tile.compressed_length]

    def payloads(self) -> list[bytes]:
        """Every tile's payload bytes, in directory order."""
        return [self.payload(tile) for tile in self.tiles]


def read_container(path: Path) -> Container:
    """Read a container from a file."""
    return Container(path.read_bytes())


def expected_rect(
    rgba: bytes, image_width: int, x: int, y: int, width: int, height: int
) -> bytes:
    """The part of an RGBA8 image a rectangle covers, row by row.

    The rows of a rectangle are not contiguous in the whole image's buffer,
    so a rectangle is taken one row at a time.
    """
    out = bytearray()
    row_bytes = width * 4
    for row in range(height):
        start = ((y + row) * image_width + x) * 4
        out += rgba[start : start + row_bytes]
    return bytes(out)


def rgba8_of(raw: bytes, width: int, height: int, pixel_format: str) -> bytes:
    """Convert canonical raw pixels into the RGBA8 the editor displays.

    The conversion is the one the editor uses: a grey sample is replicated
    into the three colour channels, and a picture with no alpha channel is
    shown opaque. It is written here from the rules, so the measurement
    checks the editor against an independent implementation of them.
    """
    out = bytearray(width * height * 4)
    count = width * height
    if pixel_format == "gray8":
        for index in range(count):
            value = raw[index]
            base = index * 4
            out[base] = value
            out[base + 1] = value
            out[base + 2] = value
            out[base + 3] = 255
    elif pixel_format == "graya8":
        for index in range(count):
            value = raw[index * 2]
            base = index * 4
            out[base] = value
            out[base + 1] = value
            out[base + 2] = value
            out[base + 3] = raw[index * 2 + 1]
    elif pixel_format == "rgb8":
        for index in range(count):
            base = index * 4
            out[base] = raw[index * 3]
            out[base + 1] = raw[index * 3 + 1]
            out[base + 2] = raw[index * 3 + 2]
            out[base + 3] = 255
    elif pixel_format == "rgba8":
        out[:] = raw[: count * 4]
    else:
        raise ValueError(f"the editor does not show a {pixel_format} picture")
    return bytes(out)
