"""PSKX static mesh (tools/ue-export `mesh` output) -> .glb in the gun frame (x, -y, z in metres, see build_models.py).
usage: python shell_glb.py SM_Shell_9mm_Empty.pskx T_9mm_ALB.png out.glb
Used for models/shell.glb (SM_Shell_9mm_Empty, the SpawnShell default mesh)."""
import json, struct, sys

src, tex, out = sys.argv[1:4]
d = open(src, "rb").read()
chunks, p = {}, 0
while p < len(d):
    name = d[p:p + 20].split(b"\0")[0].decode()
    _, size, count = struct.unpack_from("<iii", d, p + 20)
    chunks[name] = (d[p + 32:p + 32 + size * count], size, count)
    p += 32 + size * count
pts = [struct.unpack_from("<3f", chunks["PNTS0000"][0], 12 * i) for i in range(chunks["PNTS0000"][2])]
wd, ws, wn = chunks["VTXW0000"]
wedges = [struct.unpack_from("<Iff", wd, ws * i) for i in range(wn)]
fd, fs, fn = chunks.get("FACE3200", chunks.get("FACE0000"))
fmt = "<3I" if fs >= 18 else "<3H"
tris = [struct.unpack_from(fmt, fd, fs * i) for i in range(fn)]
pos = [(pts[w[0]][0] / 100, -pts[w[0]][1] / 100, pts[w[0]][2] / 100) for w in wedges]
uv = [(w[1], w[2]) for w in wedges]
idx = [i for t in tris for i in (t[0], t[2], t[1])]  # Y mirror flips winding
nrm = [[0.0] * 3 for _ in pos]
for a, b, c in zip(idx[::3], idx[1::3], idx[2::3]):
    u = [pos[b][k] - pos[a][k] for k in range(3)]
    v = [pos[c][k] - pos[a][k] for k in range(3)]
    n = (u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0])
    for i in (a, b, c):
        for k in range(3):
            nrm[i][k] += n[k]
for n in nrm:
    l = sum(x * x for x in n) ** 0.5 or 1.0
    n[:] = [x / l for x in n]
lo = [min(p[k] for p in pos) for k in range(3)]
hi = [max(p[k] for p in pos) for k in range(3)]
print("bounds m", lo, hi, "verts", len(pos), "tris", len(idx) // 3)
bin_ = b"".join(struct.pack("<3f", *p) for p in pos) + b"".join(struct.pack("<3f", *n) for n in nrm) + b"".join(struct.pack("<2f", u, v) for u, v in uv) + struct.pack(f"<{len(idx)}I", *idx)
png = open(tex, "rb").read()
o1, o2, o3, o4 = 0, len(pos) * 12, len(pos) * 24, len(pos) * 32
png_off = len(bin_)
bin_ += png + b"\0" * (-len(png) % 4)
g = {
    "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
    "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}],
    "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicFactor": 0.8, "roughnessFactor": 0.4}}],
    "textures": [{"source": 0}], "images": [{"bufferView": 4, "mimeType": "image/png"}],
    "accessors": [
        {"bufferView": 0, "componentType": 5126, "count": len(pos), "type": "VEC3", "min": lo, "max": hi},
        {"bufferView": 1, "componentType": 5126, "count": len(pos), "type": "VEC3"},
        {"bufferView": 2, "componentType": 5126, "count": len(pos), "type": "VEC2"},
        {"bufferView": 3, "componentType": 5125, "count": len(idx), "type": "SCALAR"}],
    "bufferViews": [{"buffer": 0, "byteOffset": o, "byteLength": l} for o, l in ((o1, o2 - o1), (o2, o3 - o2), (o3, o4 - o3), (o4, len(idx) * 4))] + [{"buffer": 0, "byteOffset": png_off, "byteLength": len(png)}],
    "buffers": [{"byteLength": len(bin_)}]}
j = json.dumps(g).encode()
j += b" " * (-len(j) % 4)
open(out, "wb").write(struct.pack("<4sII", b"glTF", 2, 28 + len(j) + len(bin_)) + struct.pack("<I4s", len(j), b"JSON") + j + struct.pack("<I4s", len(bin_), b"BIN\0") + bin_)
