"""assets-local/textures/fx/vhs_01_atlas.png (8192x6480 luma) -> vhs_01_atlas.ktx2 as R8.

Bevy decodes a luma PNG to RGBA8 (202 MB VRAM + a slow 53 MP decode at startup); vhs.wesl only reads .r, so R8 (53 MB) is
lossless for it. transition.rs prefers the .ktx2 when present.   python tools/bake_vhs.py [assets-local]
"""
import struct, sys
import numpy as np
from PIL import Image

root = sys.argv[1] if len(sys.argv) > 1 else "assets-local"
im = Image.open(f"{root}/textures/fx/vhs_01_atlas.png").convert("L")
w, h = im.size
data = np.asarray(im, np.uint8).tobytes()
# basic DFD, R8 unorm linear: 1 sample, 8 bits, channel R
dfd = struct.pack("<11I", 44, 0, 2 | (40 << 16), 1 | (1 << 8) | (1 << 16), 0, 1, 0, 7 << 16, 0, 0, 255)
dfd_off = 80 + 24
head = b"\xabKTX 20\xbb\r\n\x1a\n" + struct.pack("<9I", 9, 1, w, h, 0, 0, 1, 1, 0)  # VK_FORMAT_R8_UNORM
head += struct.pack("<4I2Q", dfd_off, len(dfd), 0, 0, 0, 0)
pos = (dfd_off + len(dfd) + 7) // 8 * 8
out = head + struct.pack("<3Q", pos, len(data), len(data)) + dfd + b"\0" * (pos - dfd_off - len(dfd)) + data
open(f"{root}/textures/fx/vhs_01_atlas.ktx2", "wb").write(out)
print(w, h, len(out) >> 20, "MB")
