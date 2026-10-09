"""Decode an uncompressed (BGRA8 / G8) cooked Texture2D whose single mip is inline in the .uexp, to PNG.
Usage: python -I tools/raw_tex.py <file.uexp> <width> <height> <bpp 4|1> <out.png>"""
import struct, sys, zlib
path, w, h, bpp, out = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
d = open(path, 'rb').read()
n = w * h * bpp
start = len(d) - 4 - n  # pixel data is the tail of the export, before the 4-byte package tag
assert d[-4:] == bytes.fromhex('c1832a9e'), 'unexpected uexp footer'
px = d[start:start + n]
rows = []
for y in range(h):
    r = px[y * w * bpp:(y + 1) * w * bpp]
    if bpp == 4:  # BGRA -> RGBA
        b = bytearray(r); b[0::4], b[2::4] = r[2::4], r[0::4]; r = bytes(b)
    rows.append(b'\x00' + r)
def chunk(t, data): return struct.pack('>I', len(data)) + t + data + struct.pack('>I', zlib.crc32(t + data) & 0xffffffff)
ihdr = struct.pack('>IIBBBBB', w, h, 8, 6 if bpp == 4 else 0, 0, 0, 0)
open(out, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', ihdr) + chunk(b'IDAT', zlib.compress(b''.join(rows), 6)) + chunk(b'IEND', b''))
print('ok', out, w, h)
