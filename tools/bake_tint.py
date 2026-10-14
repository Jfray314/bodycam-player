"""Bakes the operator-skin "Color" materials (MI_*_Color*: grey detail Diffuse + IDMap channels selecting tinted tile patterns)
into a plain base-colour texture, so build_glb (which takes PM_Diffuse) doesn't render the grey detail map as white gear.
Usage: python -I tools/bake_tint.py <_work/<item>/<item>.json>   (rewrites PM_Diffuse to <name>_tinted.png; idempotent)
ponytail: the UE material graph is compiled away; colour = sum(ID channel i * tint_i * tile_i / mean(tile_i)) * Diffuse / mean(Diffuse),
with the uncovered remainder using Tile A. Good enough for near-black gear; refit against in-game captures if colours look off."""
import json, os, sys
import numpy as np
from PIL import Image

path = sys.argv[1]
d = os.path.dirname(os.path.abspath(path))
meta = json.load(open(path))


def load(name, size=None):
    im = Image.open(os.path.join(d, name)).convert("RGBA")
    if size and im.size != size:
        im = im.resize(size, Image.BILINEAR)
    return np.asarray(im, dtype=np.float32) / 255.0


def srgb_to_lin(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def lin_to_srgb(c):
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(np.clip(c, 0, None), 1 / 2.4) - 0.055)


n = 0
for m in meta["materials"]:
    tex, cols, sc = m["textures"], m.get("colors", {}), m.get("scalars", {})
    dif = tex.get("Diffuse") or tex.get("PM_Diffuse")
    if "IDMap" not in tex or "Tile A tint" not in cols or not dif:
        continue
    base = load(dif)
    h, w = base.shape[:2]
    idm = load(tex["IDMap"], (w, h))
    detail = srgb_to_lin(base[..., :3]).mean(-1, keepdims=True)
    detail = detail / max(detail.mean(), 1e-3)
    out = np.zeros((h, w, 3), np.float32)
    cover = np.zeros((h, w, 1), np.float32)
    for i, k in enumerate("ABCD"):
        tint = cols.get(f"Tile {k} tint")
        if tint is None:
            continue
        wgt = idm[..., i : i + 1] if i < 4 else 0
        tile_name = tex.get(f"Tile {k}")
        pat = np.ones((h, w, 1), np.float32)
        if tile_name:
            t = srgb_to_lin(load(tile_name)[..., :3]).mean(-1)
            s = sc.get(f"Tile {k} scale", 1.0)
            ys, xs = (np.arange(h) * s * t.shape[0] / h).astype(int) % t.shape[0], (np.arange(w) * s * t.shape[1] / w).astype(int) % t.shape[1]
            pat = (t[np.ix_(ys, xs)] / max(t.mean(), 1e-3))[..., None]
        out += wgt * np.array(tint[:3], np.float32) * pat
        cover += wgt
    out += np.clip(1 - cover, 0, 1) * np.array(cols["Tile A tint"][:3], np.float32)
    out *= detail
    name = os.path.splitext(dif)[0] + "_tinted.png"
    Image.fromarray((np.clip(lin_to_srgb(out), 0, 1) * 255).astype(np.uint8)).save(os.path.join(d, name))
    tex["PM_Diffuse"] = name
    n += 1
json.dump(meta, open(path, "w"), indent=1)
print(f"tinted {n} materials")
