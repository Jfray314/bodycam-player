"""Flashlight / laser attachment actors -> assets/lights.json {attachment id: spec}
python -I tools/lights.py [<extracted_json_root>]
id = slug of the DT_WeaponSkins row (as tools/attall.py); the skin's AssociatedGameplayRow in DT_WeaponItems names the actor class
(ActorClassOverride): Flashlight_C / Laser_C or one of their *_Child BPs. Spec = the base BP's component templates (Flashlight.json
MainLight / MuzzleLight SpotLights, Laser.json) overlaid with the child's InheritableComponentHandler overrides.
Laser beam colour = MI_Laser_Beam_<Colour>.Beam Color (LightProfilePack, ue-export dump; red is also the base BP's sphere material).
Light-function textures: ue-export tex Bodycam/Content/LightProfilePack/Lighting/Profiles/<name> -> assets-local/textures/light/<name>.png."""
import json, os, re, sys

root = (sys.argv[1] if len(sys.argv) > 1 else os.environ.get("BODYCAM_EXTRACTED", os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "extracted")) + "/json") + "/Content/"
rows = lambda f: json.load(open(root + "BodycamCore/ItemsDefinition/" + f))["exports"][0]["properties"]["_rows"]
skins, wi = rows("Skins/DT_WeaponSkins.json"), rows("Gameplay/DT_WeaponItems.json")
slug = lambda s: re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")
ATT = root + "BodycamWeapons/Core/Blueprint/Attachments/"

# MI_LightFunction* -> texture (MI_Flashlight_Light_Function[N].Light_Function_Texture)
LF = {"MI_Flashlight_Light_Function": "T_Flashlight_Mask_D2", "MI_Flashlight_Light_Function1": "T_Flashlight_Mask_D", "MI_Flashlight_Light_Function2": "cookie_2",
      "MI_Flashlight_Light_Function3": "t_flashlightpattern", "MI_Flashlight_Light_Function4": "telechargement__2_1"}
# MI_Laser_Beam_<c>.Beam Color (linear)
LASER = {"Red": [1.0, 0.0, 0.0], "Green": [0.0, 1.0, 0.0], "Blue": [0.244792, 0.244792, 1.0], "Yellow": [1.0, 0.853228, 0.187634]}


def templates(path):
    """component name -> properties of a BP json (own templates; for a child these are only the overrides)"""
    d = json.load(open(path))
    return {e["name"].replace("_GEN_VARIABLE", ""): e.get("properties", {}) for e in d["exports"] if "_GEN_VARIABLE" in e["name"]}


def light(p):
    c = p.get("LightColor") or {"r": 255, "g": 255, "b": 255}
    f = p.get("LightFunctionMaterial")
    return {"inner": p.get("InnerConeAngle", 0.0), "outer": p.get("OuterConeAngle", 44.0), "intensity": p.get("Intensity", 0.0),
            "color": [c["r"], c["g"], c["b"]], "temp": p.get("Temperature", 0.0) if p.get("bUseTemperature") else 0.0,
            "atten": p.get("AttenuationRadius", 1000.0), "exp": p.get("LightFalloffExponent", 8.0), "radius": p.get("SourceRadius", 0.0),
            "ies": p.get("IESBrightnessScale", 1.0), "fnc": LF[f.rsplit("/", 1)[-1].split(".")[0]] if f else None}


bases = {"Flashlight": templates(ATT + "Flashlight/Flashlight.json"), "Laser": templates(ATT + "Laser/Laser.json")}
out = {}
for k, s in skins.items():
    g = wi.get(s.get("AssociatedGameplayRow", {}).get("RowName"))
    cls = (g or {}).get("ActorClassOverride", "")
    m = re.search(r"Attachments/(Flashlight|Laser)/(\w+)\.", cls)
    if not m:
        continue
    kind, bp = m.groups()
    comps = {n: dict(p) for n, p in bases[kind].items()}
    if bp != kind:
        for n, p in templates(ATT + f"{kind}/{bp}.json").items():
            comps.setdefault(n, {}).update(p)
    if kind == "Flashlight":
        out[slug(k)] = {"k": "flash", "bp": bp, "main": light(comps["MainLight"]), "muzzle": light(comps["MuzzleLight"])}
    else:
        mi = (comps["LaserBeam"].get("OverrideMaterials") or ["MI_Laser_Beam_Red"])[0]
        c = re.search(r"MI_Laser_Beam_(\w+)", mi)
        out[slug(k)] = {"k": "laser", "bp": bp, "color": LASER[c.group(1) if c else "Red"], "opacity": 0.2, "dot": 0.01, "beam_w": 2.0}
here = os.path.dirname(os.path.abspath(__file__))
json.dump(out, open(os.path.join(here, "..", "assets", "lights.json"), "w"), indent=1, sort_keys=True)
print(len(out), "specs", sorted({v["bp"] for v in out.values()}))
