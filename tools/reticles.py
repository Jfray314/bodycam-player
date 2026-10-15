"""Bake assets/reticles.json: per library optic, the reticle texture/colour/scale (MI_ReticleData_* chain) and scope magnification.
python -I tools/reticles.py [extracted_json_root]   then export the listed textures with ue-export `tex` (see REWRITE.md)."""
import json, os, sys
R = (sys.argv[1] if len(sys.argv) > 1 else "C:/code/bodycam-player/extracted/json") + "/Content/"
O = "BodycamWeapons/Attachement/Optics/"
# id -> (optic mesh json, default reticle data MI). ponytail: the game lets the player pick a reticle item; defaults are guesses.
LIB = {
    "reddot": (O + "red_dot_SigSauer_Romeo4/red_dot_SigSauer_Romeo4_GroupedParts", "BodycamScope/Reticles/Regular/MI_ReticleData_Dot_Red"),
    "holo": (O + "Wilcox_BOSS_Xe/Wilcox_BOSS_Xe", "BodycamScope/Reticles/Regular/MI_ReticleData_CircleDotMK3_Red"),
    "acog": (O + "ACOG_Rifle_Scope/ACOG_Rifle_Scope", "BodycamScope/Reticles/Magnified/MI_ReticleMagnifiedData_ChevronMK1_Red"),
    "canted": (O + "Leupold-deltapoint-45-degree/Leupold_DeltaPoint_Canted", "BodycamScope/Reticles/Regular/MI_ReticleData_Dot_Red"),
}


def props(path):
    p = R + path.split(".")[0].replace("/Game/", "") + ".json"
    return json.load(open(p))["exports"][0]["properties"] if os.path.exists(p) else None


def chain(path):
    """MI chain child-first -> merged parameter dict (child wins)."""
    out = {}
    while path and (pr := props(path)):
        for k in ("ScalarParameterValues", "VectorParameterValues", "TextureParameterValues"):
            for v in pr.get(k, []):
                out.setdefault(v["ParameterInfo"]["Name"], v["ParameterValue"])
        path = pr.get("Parent")
    return out


res = {}
for id, (mesh, ret) in LIB.items():
    s = json.dumps(json.load(open(R + mesh + ".json")))
    optic = chain(s.split('"OpticReticleMaterial": "')[1].split('"')[0]) if "OpticReticleMaterial" in s else {}
    d = chain(ret)
    tex = d.get("ReticleTexture") or optic.get("ReticleTexture")
    c = d.get("ReticleColor") or d.get("ReticleIlluminationColor") or optic.get("ReticleColor") or {"r": 1, "g": 0, "b": 0}
    so = d.get("ReticleScaleAndOffset") or optic.get("ReticleScaleAndOffset") or {"r": 1, "g": 1, "b": 0, "a": 0}
    res[id] = {
        "tex": tex.split("/")[-1].split(".")[0], "tex_path": tex,
        "color": [c["r"], c["g"], c["b"]],
        "intensity": d.get("ReticleIlluminationIntensity", d.get("ReticleIlluminationContrast", optic.get("ReticleIlluminationIntensity", 1.0))),
        "scale": [so["r"], so["g"], so["b"], so["a"]],
    }
    if "Magnification" in optic:
        res[id]["mag"] = optic["Magnification"]
        res[id]["screen_fraction"] = optic["ScreenHeightFraction"]
json.dump(res, open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets", "reticles.json"), "w"), indent=1)
print(json.dumps(res, indent=1))

# textures: export with ue-export `tex Bodycam/Content/BodycamScope/Textures/<tex_path>` into assets-local/models/attachments/reticles,
# then this turns the black-background masks into white + alpha = luminance (512 px) so they alpha-blend.
from PIL import Image  # noqa: E402

out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "assets-local", "models", "attachments", "reticles")
for tex in {r["tex"] for r in res.values()}:
    f = os.path.join(out, tex + ".png")
    if os.path.exists(f):
        im = Image.open(f).convert("RGB").resize((512, 512), Image.LANCZOS)
        a = im.convert("L").point(lambda v: min(255, v * 2))
        Image.merge("RGBA", (Image.new("L", im.size, 255),) * 3 + (a,)).save(f)
