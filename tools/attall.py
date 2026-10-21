"""Every attachment the 20 guns can take -> <work>/attall.json (ue-export `att` list) + <work>/attall_meta.json (game data).
DT_WeaponItems: an attachment fits a gun when one of its ItemCompatibilities is a prefix of the gun's ItemClassID.
Visible options are DT_WeaponSkins rows (AssociatedGameplayRow = the attachment); id = slug of the skin row.
Skeletal-mesh skins are skipped (only static meshes mount on sockets); Ammo / Reticle classes carry no geometry we model.
python -I tools/attall.py <work_dir> [<extracted_json_root>]"""
import json, os, re, sys, glob
work = sys.argv[1]
root = (sys.argv[2] if len(sys.argv) > 2 else "F:/code/bodycam-player/extracted/json") + "/Content/BodycamCore/ItemsDefinition/"
here = os.path.dirname(os.path.abspath(__file__))
rows = lambda f: json.load(open(root + f))["exports"][0]["properties"]["_rows"]
wi, skins, shop = rows("Gameplay/DT_WeaponItems.json"), rows("Skins/DT_WeaponSkins.json"), rows("DT_NewShopItem.json")
slug = lambda s: re.sub(r"[^a-z0-9]+", "_", s.lower()).strip("_")
asset = lambda p: "Bodycam/Content/" + p[len("/Game/"):].rsplit(".", 1)[0]
SKIP = ("Ammo", "Reticle")
guns = [json.load(open(f))["row"] for f in glob.glob(os.path.join(here, "..", "assets", "weapons", "*.json"))]
by_gp = {}
for k, s in skins.items():
    by_gp.setdefault(s.get("AssociatedGameplayRow", {}).get("RowName"), []).append(k)
items, compat, defaults, export, seen = {}, {}, {}, [], {}
for g in guns:
    cls = wi[g]["ItemClassID"]
    ids = []
    for gp, v in wi.items():
        c = v.get("ItemClassID", "")
        if not c.startswith("Item.Attachment.") or c[16:].startswith(SKIP) or not any(cls.startswith(p) for p in v.get("ItemCompatibilities", [])):
            continue
        for sk in by_gp.get(gp, []):
            s = skins[sk]
            if not s.get("StaticMesh"):
                continue
            i = slug(sk)
            if i not in items:
                mats = [m or "" for m in s.get("Materials", [])]  # per material slot; "" keeps the mesh default
                key = (s["StaticMesh"], tuple(mats))
                if key not in seen:  # one export per mesh + material set
                    seen[key] = i
                    export.append({"id": i, "kind": c[16:], "row": sk, "mesh": asset(s["StaticMesh"]), "materials": [asset(m) if m else "" for m in mats]})
                items[i] = {"skin": sk, "gp": gp, "cls": c[16:], "label": (s["ItemName"].get("text") if isinstance(s.get("ItemName"), dict) else s.get("ItemName")) or sk, "glb": seen[key],
                            "icon": (s.get("ItemIcons") or [""])[0].rsplit(".", 1)[0].rsplit("/", 1)[-1],
                            "icon_path": asset((s.get("ItemIcons") or [""])[0]) if (s.get("ItemIcons") or [""])[0] else "",
                            "effects": [e.rsplit(".", 1)[0].rsplit("/", 1)[-1] for e in v.get("GameplayDataDefinition", {}).get("GameplayEffectsModifier", [])]}
            ids.append(i)
    compat[g] = sorted(set(ids))
    defaults[g] = [slug(shop[d["RowName"]]["AssociatedItemSkinRow"]["RowName"]) for d in wi[g].get("DefaultAttachments", []) if d["RowName"] in shop]
os.makedirs(work, exist_ok=True)
json.dump(export, open(os.path.join(work, "attall.json"), "w"), indent=1)
json.dump({"items": items, "compat": compat, "defaults": defaults}, open(os.path.join(work, "attall_meta.json"), "w"), indent=1)
print(len(items), "skins,", len(export), "meshes to export;", {g: len(v) for g, v in compat.items()})
