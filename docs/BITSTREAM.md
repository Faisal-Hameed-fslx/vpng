# Bitstream reference

This is the byte-level companion to `docs/VPNG_SPEC_0_1.md`. It gives the exact
byte sequence of a minimal, fully worked file so an implementer can check a
reader against it by hand.

## Worked example: 1 × 1 gray8

An image of one pixel, value `0x00`, one 256×256 nominal tile (which is clipped
to 1×1), Raw codec. Total file size: `64 + 40 + 4 + 1 = 109` bytes.

```text
offset  bytes                       meaning
------  --------------------------  ---------------------------------------
0       56 50 4E 47 0D 0A 1A 0A     magic "VPNG\r\n\x1a\n"
8       00 00                       major version 0
10      01 00                       minor version 1
12      40 00 00 00                 header size 64
16      01 00 00 00                 width 1
20      01 00 00 00                 height 1
24      01                          channels 1
25      08                          bits per channel 8
26      00                          color model 0 (Gray)
27      00                          default codec 0 (Raw)
28      01 00 00 00                 tile count 1
32      40 00 00 00 00 00 00 00     directory offset 64
40      6D 00 00 00 00 00 00 00     declared file size 109
48      00 00 00 00                 feature flags 0
52      ?? ?? ?? ??                 header CRC32C over [0..52) ++ [56..64)
56      00 00 00 00 00 00 00 00     reserved, zero

64      00 00 00 00                 tile x = 0
68      00 00 00 00                 tile y = 0
72      01 00                       tile width = 1
74      01 00                       tile height = 1
76      00                          codec id 0
77      00                          predictor id 0
78      00 00                       flags 0
80      6C 00 00 00 00 00 00 00     payload offset 108
88      01 00 00 00 00 00 00 00     compressed length 1
96      01 00 00 00                 uncompressed length 1
100     ?? ?? ?? ??                 payload CRC32C
104     ?? ?? ?? ??                 directory CRC32C over bytes [64..104)

108     00                          payload: the single gray sample
```

`??` marks values that are computed rather than fixed; the layout and every
other field is exact. The golden test `header_layout_is_pinned` asserts the same
facts against the encoder's real output.

## Worked example: 8 × 1 gray8, Palette

An image of eight gray pixels `AA 55 AA 55 AA 55 AA 55`, one 256×256 nominal
tile (clipped to 8×1), encoded as Palette. Raw would need 8 payload bytes; the
palette needs 5, so this is a case an auto-selecting encoder genuinely chooses
Palette for. Two distinct values appear in first-appearance order (`0xAA` →
index 0, `0x55` → index 1), so the index width is 1 bit and the indices
`0 1 0 1 0 1 0 1` pack MSB-first into `0b0101_0101 = 0x55`. Total file size:
`64 + 40 + 4 + 5 = 113` bytes.

```text
offset  bytes                       meaning
------  --------------------------  ---------------------------------------
...     (header, as above, but)
16      08 00 00 00                 width 8
20      01 00 00 00                 height 1
27      01                          default codec 1 (Palette)
40      71 00 00 00 00 00 00 00     declared file size 113

64      00 00 00 00                 tile x = 0
68      00 00 00 00                 tile y = 0
72      08 00                       tile width = 8
74      01 00                       tile height = 1
76      01                          codec id 1 (Palette)
77      00                          predictor id 0
78      00 00                       flags 0
80      6C 00 00 00 00 00 00 00     payload offset 108
88      05 00 00 00 00 00 00 00     compressed length 5
96      08 00 00 00                 uncompressed length 8 (= 8 * 1 * 1)
100     ?? ?? ?? ??                 payload CRC32C
104     ?? ?? ?? ??                 directory CRC32C over bytes [64..104)

108     00                          palette header: width code 0 => 1 bit
109     01                          entry_count - 1 = 1  (two entries)
110     AA                          entry 0
111     55                          entry 1
112     55                          packed indices 0,1,0,1,0,1,0,1 (MSB-first)
```

The palette payload (5 bytes) is smaller than the tile's canonical bytes (8), so
an auto-selecting encoder picks it. When the palette overhead does not pay for
itself (very few or very many colors), selection falls back to Raw.

## Worked example: 8 × 1 gray8, Run-length

The same eight gray pixels all set to one value, `AA AA AA AA AA AA AA AA`, one
256×256 nominal tile (clipped to 8×1), encoded as Run-length. One repeat token
covers all eight pixels: control `0x80 | (8 - 1) = 0x87`, then the symbol `AA`.
That is 2 payload bytes, beating Raw (8) and Palette (4), so an auto-selecting
encoder chooses Run-length. Total file size: `64 + 40 + 4 + 2 = 110` bytes.

```text
offset  bytes                       meaning
------  --------------------------  ---------------------------------------
...     (header, as above, but)
16      08 00 00 00                 width 8
20      01 00 00 00                 height 1
27      02                          default codec 2 (Run-length)
40      6E 00 00 00 00 00 00 00     declared file size 110

64      00 00 00 00                 tile x = 0
68      00 00 00 00                 tile y = 0
72      08 00                       tile width = 8
74      01 00                       tile height = 1
76      02                          codec id 2 (Run-length)
77      00                          predictor id 0
78      00 00                       flags 0
80      6C 00 00 00 00 00 00 00     payload offset 108
88      02 00 00 00 00 00 00 00     compressed length 2
96      08 00 00 00                 uncompressed length 8 (= 8 * 1 * 1)
100     ?? ?? ?? ??                 payload CRC32C
104     ?? ?? ?? ??                 directory CRC32C over bytes [64..104)

108     87                          repeat token, count = 7 + 1 = 8
109     AA                          the repeated gray symbol
```

A literal token (`0x00..=0x7F`) would instead be followed by `control + 1` exact
pixel tuples; because every token carries its own count, the two forms never
need an escape byte.

## General assembly order

1. Header (64 bytes, checksum computed last).
2. Directory: `tile_count` entries in canonical order, then its CRC trailer.
3. Payloads back to back, each at the offset recorded in its entry, with its own
   CRC recorded in the entry.

## Reading order for a strict decoder

1. Total length and `declared_file_size` must agree.
2. Header fields and checksum.
3. Directory length, CRC, and entry count.
4. Per-entry: geometry, uncompressed length, codec support, payload range.
5. Payload ranges must not overlap and must start after the directory.
6. Derive the tile grid and require exact, ordered coverage.
7. Only then decode payloads, verifying each payload CRC first.
