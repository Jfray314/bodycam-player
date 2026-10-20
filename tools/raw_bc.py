"""Decode a BC7 (or BC5) mip dump from `ue-export texraw` to PNG via Pillow's DDS reader.
Usage: python tools/raw_bc.py (needs Pillow; not -I) <Name.PF_BC7.WxH.raw> <out.png>"""
import io, struct, sys
from PIL import Image
src, out = sys.argv[1], sys.argv[2]
fmt, size, _ = src.rsplit('.', 3)[-3:]
w, h = map(int, size.split('x'))
dxgi = {'PF_BC7': 98, 'PF_BC5': 83}[fmt]
d = open(src, 'rb').read()
hdr = b'DDS ' + struct.pack('<7I', 124, 0x81007, h, w, len(d), 0, 1) + bytes(44) + struct.pack('<2I4s', 32, 4, b'DX10') + bytes(20) + struct.pack("<5I", 0x1000, 0, 0, 0, 0)
hdr += struct.pack('<5I', dxgi, 3, 0, 1, 0)
Image.open(io.BytesIO(hdr + d)).convert('RGBA').save(out)
print('ok', out, w, h)
