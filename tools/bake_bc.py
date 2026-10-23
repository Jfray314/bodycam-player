"""Bake a map glb's opaque sRGB base-colour textures to BC1 + full mip chain (KTX2 inside the glb).

    python tools/bake_bc.py assets-local/maps/scene2.glb [out.glb]

Why: the lobby's ~630 1024^2 JPEG textures upload as RGBA8 with no mips (4 MB each, 2.4 GB VRAM). BC1+mips is 0.67 MB
each (~6x smaller), skips the JPEG decode at load, and Bevy (ktx2 feature) reads `image/ktx2` images in a glb directly.
Only images used solely as baseColor / emissive (sRGB) and fully opaque are converted; everything else is copied as-is.
The original is kept as <name>.orig.glb when writing in place. Needs numpy + Pillow.
"""
import io, json, shutil, struct, sys
import numpy as np
from PIL import Image

BC1_RGBA_SRGB = 134  # VK_FORMAT_BC1_RGBA_SRGB_BLOCK (Bevy picks the sRGB variant from the glTF slot)


def to_lin(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def to_srgb(c):
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(np.maximum(c, 0), 1 / 2.4) - 0.055)


def bc1(rgb):
    """rgb float32 (H, W, 3) in 0..255, H and W multiples of 4 -> BC1 bytes (4-colour mode, principal-axis endpoints)."""
    h, w, _ = rgb.shape
    x = rgb.reshape(h // 4, 4, w // 4, 4, 3).transpose(0, 2, 1, 3, 4).reshape(-1, 16, 3)
    mean = x.mean(1, keepdims=True)
    d = x - mean
    cov = np.einsum("nik,nil->nkl", d, d)
    axis = np.ones((len(x), 3), np.float32)
    for _ in range(4):
        axis = np.einsum("nkl,nl->nk", cov, axis)
        axis /= np.maximum(np.linalg.norm(axis, axis=1, keepdims=True), 1e-6)
    t = np.einsum("nik,nk->ni", d, axis)
    lo = mean[:, 0] + axis * t.min(1)[:, None]
    hi = mean[:, 0] + axis * t.max(1)[:, None]

    def q(c):
        c = np.clip(c, 0, 255) / 255.0
        return (np.rint(c[:, 0] * 31).astype(np.uint32) << 11) | (np.rint(c[:, 1] * 63).astype(np.uint32) << 5) | np.rint(c[:, 2] * 31).astype(np.uint32)

    def deq(v):
        r, g, b = (v >> 11) & 31, (v >> 5) & 63, v & 31
        return np.stack([(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)], 1).astype(np.float32)

    c0, c1 = q(hi), q(lo)
    sw = c0 < c1
    c0, c1 = np.where(sw, c1, c0), np.where(sw, c0, c1)
    a, b = deq(c0), deq(c1)
    pal = np.stack([a, b, (2 * a + b) / 3, (a + 2 * b) / 3], 1)  # N,4,3
    dist = ((x[:, :, None, :] - pal[:, None, :, :]) ** 2).sum(-1)  # N,16,4
    idx = dist.argmin(-1).astype(np.uint32)
    idx[c0 == c1] = 0
    bits = (idx << (2 * np.arange(16, dtype=np.uint32))).sum(1).astype(np.uint32)
    return np.stack([(c0 | (c1 << 16)).astype("<u4"), bits.astype("<u4")], 1).tobytes()


def ktx2_bc1(img):
    w, h = img.size
    a = np.asarray(img.convert("RGB"), np.float32)
    levels = []
    while True:
        levels.append(bc1(a))
        if min(w, h) <= 4:
            break
        lin = to_lin(a / 255.0)
        lin = lin.reshape(h // 2, 2, w // 2, 2, 3).mean((1, 3))
        a = (to_srgb(lin) * 255.0).astype(np.float32)
        w, h = w // 2, h // 2
        if w % 4 or h % 4:
            break
        if w % 8 or h % 8:  # non power-of-two chain: stop after this level
            break
    n = len(levels)
    # basic DFD for BC1 RGBA sRGB (the ktx2 reader insists on one); the vkFormat is what Bevy actually uses
    dfd = struct.pack("<11I", 44, 0, 2 | (40 << 16), 128 | (1 << 8) | (2 << 16), 3 | (3 << 8), 8, 0, 63 << 16, 0, 0, 0xFFFFFFFF)
    dfd_off = 80 + 24 * n
    head = b"\xabKTX 20\xbb\r\n\x1a\n" + struct.pack("<9I", BC1_RGBA_SRGB, 1, img.size[0], img.size[1], 0, 0, 1, n, 0)
    head += struct.pack("<4I2Q", dfd_off, len(dfd), 0, 0, 0, 0)
    pos = (dfd_off + len(dfd) + 7) // 8 * 8
    offs = [0] * n
    for i in reversed(range(n)):  # smallest level first in the file
        offs[i] = pos
        pos += len(levels[i])
    idx = b"".join(struct.pack("<3Q", offs[i], len(levels[i]), len(levels[i])) for i in range(n))
    return head + idx + dfd + b"\0" * (offs[n - 1] - dfd_off - len(dfd)) + b"".join(levels[::-1])


def main():
    src = sys.argv[1]
    dst = sys.argv[2] if len(sys.argv) > 2 else src
    raw = open(src, "rb").read()
    jl, _ = struct.unpack("<II", raw[12:20])
    j = json.loads(raw[20:20 + jl])
    bin_ = raw[20 + jl + 8:]
    uses = {}
    for m in j["materials"]:
        pm = m.get("pbrMetallicRoughness", {})
        for slot, d in [("base", pm.get("baseColorTexture")), ("emis", m.get("emissiveTexture")), ("mr", pm.get("metallicRoughnessTexture")), ("nrm", m.get("normalTexture")), ("occ", m.get("occlusionTexture"))]:
            if d:
                uses.setdefault(j["textures"][d["index"]]["source"], set()).add(slot)
    bvs = j["bufferViews"]
    new = {}
    for i, im in enumerate(j["images"]):
        if im.get("mimeType") == "image/ktx2" or not uses.get(i) or not uses[i] <= {"base", "emis"}:
            continue
        v = bvs[im["bufferView"]]
        pil = Image.open(io.BytesIO(bin_[v["byteOffset"]:v["byteOffset"] + v["byteLength"]]))
        if pil.width % 4 or pil.height % 4 or pil.width < 16 or pil.height < 16:
            continue
        if pil.mode in ("RGBA", "LA", "P") and pil.convert("RGBA").getchannel("A").getextrema()[0] != 255:
            continue
        new[im["bufferView"]] = ktx2_bc1(pil)
        im["mimeType"] = "image/ktx2"
        print(f"\rimage {i}/{len(j['images'])}", end="", flush=True)
    print()
    out, pos = bytearray(), 0
    for k, v in enumerate(bvs):
        data = new.get(k, bin_[v["byteOffset"]:v["byteOffset"] + v["byteLength"]])
        out += b"\0" * (-len(out) % 8)
        v["byteOffset"], v["byteLength"] = len(out), len(data)
        out += data
    out += b"\0" * (-len(out) % 4)
    j["buffers"][0]["byteLength"] = len(out)
    js = json.dumps(j, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    if dst == src:
        shutil.copyfile(src, src[:-4] + ".orig.glb")
    with open(dst, "wb") as f:
        f.write(b"glTF" + struct.pack("<II", 2, 12 + 8 + len(js) + 8 + len(out)))
        f.write(struct.pack("<I", len(js)) + b"JSON" + js)
        f.write(struct.pack("<I", len(out)) + b"BIN\0" + out)
    print(f"{len(new)} images -> BC1; {len(raw) >> 20} MB -> {(12 + 16 + len(js) + len(out)) >> 20} MB")


main()
