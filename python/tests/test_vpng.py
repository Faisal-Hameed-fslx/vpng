"""Tests for the VPNG Python extension.

These tests use nothing but the standard library: the PNG files are written
and read by `png_helper`, so a CI job needs no imaging dependency.

Every test asserts a property the format promises:

* the round trip is **exact** — the decoded pixels equal the original byte
  for byte;
* the output is **deterministic** — the worker count changes the speed, not
  the bytes;
* a **lossy request is refused**, never quietly ignored;
* a **failed operation leaves no partial file** behind.
"""

from __future__ import annotations

import hashlib
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import png_helper  # noqa: E402

import vpng  # noqa: E402


def gradient(width: int, height: int, channels: int = 4) -> bytes:
    """A deterministic test picture with structure the codecs can exploit."""
    data = bytearray(width * height * channels)
    index = 0
    for y in range(height):
        for x in range(width):
            data[index] = (x * 7 + y * 3) & 0xFF
            data[index + 1] = (x * 5) & 0xFF
            data[index + 2] = (y * 11) & 0xFF
            if channels == 4:
                data[index + 3] = 255
            index += channels
    return bytes(data)


def digest(path: str | os.PathLike[str]) -> str:
    """The SHA-256 of a file."""
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class VpngTestCase(unittest.TestCase):
    """The shared temporary directory and the picture every test uses."""

    def setUp(self) -> None:
        self._temp = tempfile.TemporaryDirectory()
        self.dir = Path(self._temp.name)
        self.width = 96
        self.height = 64
        self.channels = 4
        self.pixels = gradient(self.width, self.height, self.channels)
        self.png = self.dir / "input.png"
        png_helper.write_png(
            self.png, self.width, self.height, self.pixels, self.channels, 8
        )
        self.vpng = self.dir / "input.vpng"

    def tearDown(self) -> None:
        self._temp.cleanup()


class TestRoundTrip(VpngTestCase):
    """The core promise: the round trip is exact."""

    def test_decode_matches_the_original_bytes(self) -> None:
        vpng.encode(self.png, self.vpng)
        decoded = vpng.decode_to_bytes(self.vpng)
        self.assertEqual(decoded, self.pixels)

    def test_verify_reports_an_exact_match(self) -> None:
        vpng.encode(self.png, self.vpng)
        self.assertTrue(vpng.verify(self.png, self.vpng))

    def test_verify_reports_a_mismatch_as_false(self) -> None:
        other = self.dir / "other.png"
        png_helper.write_png(other, self.width, self.height, bytes(len(self.pixels)))
        vpng.encode(other, self.vpng)
        self.assertFalse(vpng.verify(self.png, self.vpng))

    def test_decode_writes_a_png_that_reads_back_identically(self) -> None:
        vpng.encode(self.png, self.vpng)
        output = self.dir / "output.png"
        vpng.decode(self.vpng, output)
        width, height, pixels, channels, depth = png_helper.read_png(output)
        self.assertEqual((width, height), (self.width, self.height))
        self.assertEqual((channels, depth), (self.channels, 8))
        self.assertEqual(pixels, self.pixels)

    def test_sixteen_bit_samples_round_trip(self) -> None:
        pixels = bytes((x * 37) & 0xFF for x in range(32 * 32 * 4 * 2))
        source = self.dir / "wide.png"
        png_helper.write_png(source, 32, 32, pixels, 4, 16)
        target = self.dir / "wide.vpng"
        vpng.encode(source, target)
        self.assertEqual(vpng.decode_to_bytes(target), pixels)

    def test_grayscale_round_trips(self) -> None:
        pixels = bytes((x * 3) & 0xFF for x in range(64 * 48))
        source = self.dir / "gray.png"
        png_helper.write_png(source, 64, 48, pixels, 1, 8)
        target = self.dir / "gray.vpng"
        vpng.encode(source, target)
        self.assertEqual(vpng.decode_to_bytes(target), pixels)
        self.assertEqual(vpng.info(target)["color_model"], "Gray")


class TestDeterminism(VpngTestCase):
    """The worker count changes the speed, never the bytes."""

    def test_worker_count_does_not_change_the_file(self) -> None:
        one = self.dir / "one.vpng"
        four = self.dir / "four.vpng"
        vpng.encode(self.png, one, workers=1)
        vpng.encode(self.png, four, workers=4)
        self.assertEqual(digest(one), digest(four))

    def test_two_runs_produce_the_same_file(self) -> None:
        first = self.dir / "first.vpng"
        second = self.dir / "second.vpng"
        vpng.encode(self.png, first)
        vpng.encode(self.png, second)
        self.assertEqual(digest(first), digest(second))

    def test_encode_pixels_takes_the_pixels_directly(self) -> None:
        target = self.dir / "pixels.vpng"
        vpng.encode_pixels(self.pixels, self.width, self.height, target)
        self.assertEqual(vpng.decode_to_bytes(target), self.pixels)

    def test_encode_pixels_makes_the_same_file_as_the_file_form(self) -> None:
        from_file = self.dir / "from-file.vpng"
        from_pixels = self.dir / "from-pixels.vpng"
        vpng.encode(self.png, from_file)
        vpng.encode_pixels(self.pixels, self.width, self.height, from_pixels)
        self.assertEqual(digest(from_file), digest(from_pixels))

    def test_encode_pixels_refuses_a_wrong_sized_buffer(self) -> None:
        with self.assertRaises(ValueError):
            vpng.encode_pixels(
                self.pixels[:-1], self.width, self.height, self.dir / "bad.vpng"
            )

    def test_the_tile_size_does_not_change_the_pixels(self) -> None:
        target = self.dir / "small.vpng"
        vpng.encode(self.png, target, tile_size=16)
        self.assertEqual(vpng.decode_to_bytes(target), self.pixels)


class TestCodecSets(VpngTestCase):
    """The stable and extended codec sets, and the version they imply."""

    def test_the_default_is_a_v0_1_file(self) -> None:
        vpng.encode(self.png, self.vpng)
        info = vpng.info(self.vpng)
        self.assertEqual(info["minor"], 1)
        self.assertEqual(info["major"], 0)

    def test_the_extended_set_may_write_a_v0_3_file(self) -> None:
        vpng.encode(self.png, self.vpng, codec_set="extended")
        info = vpng.info(self.vpng)
        self.assertGreaterEqual(info["minor"], 1)
        # Whatever the version, the pixels are still exact.
        self.assertTrue(vpng.verify(self.png, self.vpng))

    def test_a_named_codec_round_trips(self) -> None:
        for codec in ("raw", "rle", "predictor"):
            with self.subTest(codec=codec):
                target = self.dir / f"{codec}.vpng"
                vpng.encode(self.png, target, codec=codec, codec_set="extended")
                self.assertEqual(vpng.decode_to_bytes(target), self.pixels)

    def test_the_palette_codec_round_trips_a_picture_it_can_hold(self) -> None:
        # The palette codec refuses a tile with more than 256 distinct values:
        # that is a typed refusal, not a lossy fallback.
        pixels = bytes([0, 0, 0, 255, 255, 0, 0, 255] * (16 * 16 // 2))
        source = self.dir / "flat.png"
        png_helper.write_png(source, 16, 16, pixels, 4, 8)
        target = self.dir / "flat.vpng"
        vpng.encode(source, target, codec="palette", codec_set="extended")
        self.assertEqual(vpng.decode_to_bytes(target), pixels)

    def test_the_palette_codec_refuses_a_picture_it_cannot_hold(self) -> None:
        with self.assertRaises(vpng.UnsupportedFeatureError):
            vpng.encode(self.png, self.vpng, codec="palette", codec_set="extended")

    def test_an_unknown_codec_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            vpng.encode(self.png, self.vpng, codec="jpeg")

    def test_an_unknown_codec_set_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            vpng.encode(self.png, self.vpng, codec_set="future")


class TestLosslessness(VpngTestCase):
    """A lossy request is refused, never quietly ignored."""

    def test_lossless_false_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            vpng.encode(self.png, self.vpng, lossless=False)
        self.assertFalse(self.vpng.exists())

    def test_a_refused_encode_leaves_no_partial_file(self) -> None:
        with self.assertRaises(ValueError):
            vpng.encode(self.png, self.vpng, lossless=False)
        self.assertEqual(list(self.dir.glob("*.tmp*")), [])

    def test_a_zero_tile_size_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            vpng.encode(self.png, self.vpng, tile_size=0)

    def test_a_zero_worker_count_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            vpng.encode(self.png, self.vpng, workers=0)


class TestIntegrity(VpngTestCase):
    """The checks and the information a file carries."""

    def test_check_passes_on_a_good_file(self) -> None:
        vpng.encode(self.png, self.vpng)
        report = vpng.check(self.vpng)
        self.assertTrue(report.ok)
        self.assertGreater(report.tiles_checked, 0)

    def test_info_reports_the_geometry(self) -> None:
        vpng.encode(self.png, self.vpng)
        info = vpng.info(self.vpng)
        self.assertEqual(info["width"], self.width)
        self.assertEqual(info["height"], self.height)
        self.assertEqual(info["color_model"], "RGBA")
        self.assertEqual(info["channels"], 4)
        self.assertEqual(info["bits_per_channel"], 8)
        self.assertEqual(info["level_count"], 1)

    def test_a_corrupt_file_is_reported_as_corrupt(self) -> None:
        vpng.encode(self.png, self.vpng)
        body = bytearray(self.vpng.read_bytes())
        body[-1] ^= 0xFF  # corrupt the last payload byte
        broken = self.dir / "broken.vpng"
        broken.write_bytes(bytes(body))
        with self.assertRaises(vpng.VpngError):
            vpng.check(broken)

    def test_a_corrupt_file_is_also_a_value_error(self) -> None:
        vpng.encode(self.png, self.vpng)
        body = bytearray(self.vpng.read_bytes())
        body[-1] ^= 0xFF
        broken = self.dir / "broken.vpng"
        broken.write_bytes(bytes(body))
        with self.assertRaises(ValueError):
            vpng.check(broken)

    def test_a_missing_file_raises_an_io_error(self) -> None:
        with self.assertRaises(OSError):
            vpng.check(self.dir / "does-not-exist.vpng")

    def test_the_exception_kinds_are_distinct(self) -> None:
        kinds = [
            vpng.InvalidArgumentError,
            vpng.UnsupportedFeatureError,
            vpng.CorruptFileError,
            vpng.ResourceLimitError,
            vpng.SizeLimitExceededError,
            vpng.VerificationFailedError,
            vpng.VpngIOError,
            vpng.InternalError,
        ]
        for kind in kinds:
            with self.subTest(kind=kind.__name__):
                self.assertTrue(issubclass(kind, vpng.VpngError))


class TestEditing(VpngTestCase):
    """Region reading, transactions, and commits."""

    def test_read_region_returns_the_right_bytes(self) -> None:
        vpng.encode(self.png, self.vpng)
        with vpng.open_document(self.vpng) as doc:
            region = doc.read_region(x=0, y=0, width=8, height=4)
            self.assertEqual(region.width, 8)
            self.assertEqual(region.height, 4)
            self.assertEqual(region.format, "rgba8")
            expected = b"".join(
                self.pixels[(y * self.width + x) * 4 : (y * self.width + x) * 4 + 4]
                for y in range(4)
                for x in range(8)
            )
            self.assertEqual(bytes(region), expected)

    def test_a_committed_edit_is_written_to_the_file(self) -> None:
        vpng.encode(self.png, self.vpng)
        with vpng.open_document(self.vpng) as doc:
            with doc.edit() as tx:
                tx.write_region(bytes(4 * 4 * 4), x=0, y=0, width=4, height=4)
                report = tx.commit()
            self.assertGreater(report.tiles_rewritten, 0)
        after = vpng.decode_to_bytes(self.vpng)

        # The region is 4x4 pixels, so it covers four rows of four pixels.
        # The buffer is tightly packed, so those pixels are not contiguous.
        expected = bytearray(self.pixels)
        for row in range(4):
            start = (row * self.width) * 4
            expected[start : start + 4 * 4] = bytes(4 * 4)
        self.assertEqual(after, bytes(expected))

    def test_an_uncommitted_transaction_is_rolled_back(self) -> None:
        vpng.encode(self.png, self.vpng)
        before = digest(self.vpng)
        with vpng.open_document(self.vpng) as doc:
            with doc.edit() as tx:
                tx.write_region(bytes(4 * 4 * 4), x=0, y=0, width=4, height=4)
                # No commit: leaving the block must roll the edit back.
        self.assertEqual(digest(self.vpng), before)

    def test_a_rolled_back_transaction_leaves_the_file_alone(self) -> None:
        vpng.encode(self.png, self.vpng)
        before = digest(self.vpng)
        with vpng.open_document(self.vpng) as doc:
            with doc.edit() as tx:
                tx.write_region(bytes(4 * 4 * 4), x=0, y=0, width=4, height=4)
                tx.rollback()
        self.assertEqual(digest(self.vpng), before)

    def test_untouched_tiles_are_copied_not_re_encoded(self) -> None:
        vpng.encode(self.png, self.vpng, tile_size=16)
        with vpng.open_document(self.vpng) as doc:
            with doc.edit() as tx:
                tx.write_region(bytes(4 * 4 * 4), x=0, y=0, width=4, height=4)
                report = tx.commit()
            self.assertEqual(report.tiles_rewritten, 1)
            self.assertGreater(report.tiles_copied, 0)

    def test_undo_and_redo_work(self) -> None:
        vpng.encode(self.png, self.vpng)
        with vpng.open_document(self.vpng) as doc:
            doc.write_region(bytes(4 * 4 * 4), x=0, y=0, width=4, height=4)
            self.assertTrue(doc.is_dirty)
            self.assertTrue(doc.undo())
            self.assertFalse(doc.is_dirty)
            self.assertTrue(doc.redo())
            self.assertTrue(doc.is_dirty)

    def test_a_region_can_be_written_back_where_it_came_from(self) -> None:
        vpng.encode(self.png, self.vpng)
        with vpng.open_document(self.vpng) as doc:
            region = doc.read_region(x=8, y=8, width=8, height=8)
            with doc.edit() as tx:
                tx.write_region(region)
                tx.commit()
        self.assertEqual(vpng.decode_to_bytes(self.vpng), self.pixels)

    def test_raw_bytes_need_a_shape(self) -> None:
        vpng.encode(self.png, self.vpng)
        with vpng.open_document(self.vpng) as doc:
            with self.assertRaises(TypeError):
                doc.write_region(bytes(16))

    def test_raw_bytes_use_the_document_s_own_format(self) -> None:
        # A grayscale document has one channel per pixel, not four. The raw
        # bytes must be read in the document's format, or the write is
        # silently the wrong shape.
        pixels = bytes((x * 3) & 0xFF for x in range(16 * 16))
        source = self.dir / "gray.png"
        png_helper.write_png(source, 16, 16, pixels, 1, 8)
        target = self.dir / "gray.vpng"
        vpng.encode(source, target)

        with vpng.open_document(target) as doc:
            self.assertEqual(doc.format, "gray8")
            with doc.edit() as tx:
                tx.write_region(bytes(4 * 4), x=0, y=0, width=4, height=4)
                tx.commit()

        after = vpng.decode_to_bytes(target)
        expected = bytearray(pixels)
        for row in range(4):
            start = row * 16
            expected[start : start + 4] = bytes(4)
        self.assertEqual(after, bytes(expected))

    def test_raw_bytes_of_the_wrong_size_are_refused(self) -> None:
        vpng.encode(self.png, self.vpng)
        with vpng.open_document(self.vpng) as doc:
            with self.assertRaises(ValueError):
                doc.write_region(bytes(16), x=0, y=0, width=4, height=4)


class TestPyramid(VpngTestCase):
    """Preview levels."""

    def test_a_pyramid_has_more_levels(self) -> None:
        vpng.encode(self.png, self.vpng, pyramid_levels=2)
        info = vpng.info(self.vpng)
        self.assertGreater(info["level_count"], 1)
        self.assertGreaterEqual(info["minor"], 2)

    def test_the_top_level_is_smaller(self) -> None:
        vpng.encode(self.png, self.vpng, pyramid_levels=2)
        top = vpng.decode_to_bytes(self.vpng, level=1)
        self.assertLess(len(top), len(self.pixels))

    def test_the_base_level_is_still_exact(self) -> None:
        vpng.encode(self.png, self.vpng, pyramid_levels=2)
        self.assertEqual(vpng.decode_to_bytes(self.vpng, level=0), self.pixels)
        self.assertTrue(vpng.verify(self.png, self.vpng))


class TestModuleSurface(VpngTestCase):
    """The names the package promises."""

    def test_the_constants_are_present(self) -> None:
        self.assertEqual(vpng.FORMAT_VERSION, "0.3")
        self.assertEqual(vpng.DEFAULT_TILE_SIZE, 256)
        self.assertIn("rgba8", vpng.FORMATS)
        self.assertIn(6, vpng.CODECS)
        self.assertEqual(vpng.CODEC_SETS, ("stable", "extended"))

    def test_pixel_format_normalises_a_name(self) -> None:
        self.assertEqual(vpng.pixel_format("RGBA8"), "rgba8")
        self.assertEqual(vpng.pixel_format("rgb16"), "rgb16")
        with self.assertRaises(ValueError):
            vpng.pixel_format("cmyk")

    def test_the_package_is_typed(self) -> None:
        marker = Path(vpng.__file__).parent / "py.typed"
        self.assertTrue(marker.exists())


if __name__ == "__main__":
    unittest.main()
