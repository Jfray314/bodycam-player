"""assets/weapons/<Row>.json default_attachments -> DT_NewShopItem -> DT_WeaponSkins (mesh) / DT_WeaponItems (class) -> <work>/attachments.json
python -I tools/attachments.py <work_dir> [<extracted_json_root>]
Receiver = skeletal mesh of the '<Gun> Base Receiver' row; parts = static meshes with the socket name derived from ItemClassID.
Ammo and reticle/optic-dot classes are skipped (not visible geometry we model)."""
import json, os, sys, glob

work = sys.argv[1]
root = (sys.argv[2] if len(sys.argv) > 2 else "F:/code/bodycam-player/extracted/json") + "/Content/BodycamCore/ItemsDefinition/"
here = os.path.dirname(os.path.abspath(__file__))
weapons = os.path.join(here, "..", "assets", "weapons")
rows = lambda f: json.load(open(root + f))["exports"][0]["properties"]["_rows"]
shop, skins, wi = rows("DT_NewShopItem.json"), rows("Skins/DT_WeaponSkins.json"), rows("Gameplay/DT_WeaponItems.json")
GUN_NAME = {"AK47": "Akm", "Mossberg": "Mossberg590", "SCAR": "SCAR-H", "Fostech": "Vaiga", "kriss_vector": "Vector", "BC5": "MP5", "DRAGUNOV": "SVD"}  # assets/weapons row file -> glb gun name
norm = lambda k: k.lower().replace("_", " ").replace(" base ", " ")
shop_n = {norm(k): v for k, v in shop.items()}  # default_attachments names have case/underscore typos ("UZI Magazine" vs "Uzi Magazine", "Remington700_base_Barrel_Short")
wi_n = {norm(k): v for k, v in wi.items()}
SKIP = ("Ammo", "Reticle", "Optic")
asset = lambda p: ("Bodycam/Content/" + p[len("/Game/"):]).rsplit(".", 1)[0] if p.startswith("/Game/") else ""

out = {}
for f in sorted(glob.glob(os.path.join(weapons, "*.json"))):
    row = os.path.splitext(os.path.basename(f))[0]
    d = json.load(open(f))
    e = {"receiver": None, "parts": []}
    for n in d.get("default_attachments", []):
        s = shop.get(n) or shop_n.get(norm(n))
        sk = skins.get(s["AssociatedItemSkinRow"]["RowName"]) if s else None
        if not sk:
            continue
        cls = (wi.get(sk["AssociatedGameplayRow"]["RowName"]) or wi_n.get(norm(sk["AssociatedGameplayRow"]["RowName"])) or {}).get("ItemClassID", "")
        if sk.get("SkeletalMesh") and not cls.startswith("Item.Attachment"):
            e["receiver"] = asset(sk["SkeletalMesh"])
        elif cls.startswith("Item.Attachment.") and sk.get("StaticMesh") and not cls.split(".")[-1].startswith(SKIP):
            e["parts"].append({"row": n, "cls": cls, "socket": cls.split(".")[-1].replace(" ", "_"), "mesh": asset(sk["StaticMesh"])})
    out[GUN_NAME.get(row, row)] = e
# Perk items (drone): DT_PerkItems default attachments -> DT_PerkSkins meshes (Frame = receiver, Rotor/Propeller/Antenna/FPV Camera/Led/Nut = parts)
pi, ps = rows("Gameplay/DT_PerkItems.json"), rows("Skins/DT_PerkSkins.json")
# DT_BasicBundles "<Row> Basic Bundle" adds the rest (Kamikaze: battery + Long Range Device Explosive (PG-9 warhead); Sentinel: its frame + battery).
bundles = rows("DT_BasicBundles.json")
for prow in ("Kamikaze", "Sentinel"):
    e = {"receiver": None, "parts": []}
    names = [a["RowName"] for a in pi[prow]["DefaultAttachments"]]
    names += [a["RowName"] for a in bundles[prow + " Basic Bundle"]["ShopBundleRows"] if a["RowName"] not in names]
    for n in names:
        sk = ps.get(shop[n]["AssociatedItemSkinRow"]["RowName"])
        cls = pi.get(sk["AssociatedGameplayRow"]["RowName"], {}).get("ItemClassID", "") if sk else ""
        if sk and sk.get("SkeletalMesh"):
            e["receiver"] = asset(sk["SkeletalMesh"])
        elif sk and cls.startswith("Item.Attachment.") and sk.get("StaticMesh") and not cls.endswith(".Nut"):  # nuts have no socket
            e["parts"].append({"row": n, "cls": cls, "socket": cls.split(".")[-1].replace(" ", "_"), "mesh": asset(sk["StaticMesh"])})
    out[prow] = e
os.makedirs(work, exist_ok=True)
json.dump(out, open(os.path.join(work, "attachments.json"), "w"), indent=1)
print({k: (v["receiver"], len(v["parts"])) for k, v in out.items()})
