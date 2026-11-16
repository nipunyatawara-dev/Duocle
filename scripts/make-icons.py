"""Generate Duocle's original app icon and Windows icon resource."""
import math
import pathlib
import struct
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent


def png(size):
    def chunk(tag, contents):
        return struct.pack(">I", len(contents)) + tag + contents + struct.pack(">I", zlib.crc32(tag + contents))

    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            px, py = (x + .5) * 128 / size, (y + .5) * 128 / size
            rounded = math.hypot(max(abs(px - 64) - 40, 0), max(abs(py - 64) - 40, 0))
            alpha = int(255 * max(0, min(1, (21 - rounded) * size / 128)))
            glow = max(0, 1 - math.hypot(px - 95, py - 22) / 130)
            base = (int(19 + 21 * glow), int(25 + 37 * glow), int(39 + 39 * glow))
            left = abs(math.hypot((px - 55) / 1.05, py - 64) - 30)
            right = abs(math.hypot((px - 78) / 1.05, py - 64) - 21)
            dot = math.hypot(px - 62, py - 64)
            mark = max(0, min(1, max(3.4 - left, 3.0 - right, 5 - dot) * size / 128))
            front = (149, 237, 205) if right < left or dot < 5 else (239, 249, 244)
            rows.extend([round(a * (1 - mark) + b * mark) for a, b in zip(base, front)])
            rows.append(alpha)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


def ico():
    sizes = (16, 24, 32, 48, 64, 128, 256)
    offset = 6 + 16 * len(sizes)
    directory = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    images = bytearray()
    for size in sizes:
        data = png(size)
        directory.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
        images.extend(data)
    return directory + images


if __name__ == "__main__":
    (ROOT / "assets/duocle.png").write_bytes(png(512))
    (ROOT / "assets/duocle.ico").write_bytes(ico())
