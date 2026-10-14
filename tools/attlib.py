"""Curated attachment library -> <work>/attlib.json for `ue-export att` (meshes by skin StaticMesh basename).
python -I tools/attlib.py <work_dir> [<extracted_json_root>]"""
import json, os, sys
work = sys.argv[1]
root = (sys.argv[2] if len(sys.argv) > 2 else "C:/code/bodycam-player/extracted/json") + "/Content/BodycamCore/ItemsDefinition/"
skins = json.load(open(root + "Skins/DT_WeaponSkins.json"))["exports"][0]["properties"]["_rows"]
# id -> (static mesh basename, kind)
LIB = {
    "reddot": ("red_dot_SigSauer_Romeo4_GroupedParts", "optic"),
    "holo": ("Wilcox_BOSS_Xe", "optic"),
    "acog": ("ACOG_Rifle_Scope", "magnified"),
    "canted": ("Leupold_DeltaPoint_Canted", "canted"),
    "suppressor": ("silencer_M4SD", "muzzle"),
    "laser": ("MAWL_C1_LASER", "laser"),
    "peq": ("eotech_OGL_GroupedParts", "laser"),
    "flashlight": ("Surefire_X300V", "flashlight"),
    "foregrip": ("foregrip_magpul_AFG", "underbarrel"),
}
by = {}
for k, s in skins.items():
    m = s.get("StaticMesh", "")
    if m: by.setdefault(m.split("/")[-1].split(".")[0], (k, m))
out = []
for i, (mesh, kind) in LIB.items():
    r = by.get(mesh)
    if not r: print("MISSING", i, mesh); continue
    out.append({"id": i, "kind": kind, "row": r[0], "mesh": "Bodycam/Content/" + r[1][len("/Game/"):].rsplit(".", 1)[0]})
os.makedirs(work, exist_ok=True)
json.dump(out, open(os.path.join(work, "attlib.json"), "w"), indent=1)
print(len(out), "entries")
