"""Level ambient sounds -> assets-local/maps/ambients.json.

Runs `ue-export dump` on the lobby umap and its always-loaded sublevels (same list as the scene export) and collects every
placed AmbientSound actor plus BP actors whose class holds an AudioComponent with a Sound (ceiling fans). Positions are UE cm
(world, sublevels are identity); audio.rs converts with p' = (y, z, -x) * 0.01.
usage: python -I tools/ambients.py [ue-export run.sh dir]
"""
import json, subprocess, sys, os, re

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GAME = os.environ.get("BODYCAM_GAME", r"C:\Program Files (x86)\Steam\steamapps\common\Bodycam")
EXTRACTED = os.environ.get("BODYCAM_EXTRACTED", ROOT + "/extracted")
ASSETS = os.environ.get("BODYCAM_ASSETS", ROOT + "/assets-local")
DOTNET = os.environ.get("DOTNET", os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe"))
EXE = os.path.join(ROOT, "tools", "ue-export", "bin", "Release", "net10.0", "ue-export.dll")
OUT = ASSETS + "/maps/ambients.json"
LEVELS = ["Map/Lobby/Lobby", "Map/Lobby/Weapon_Room_NoWeapon", "Map/Lobby/Training_RoomB", "Map/Lobby/Loadout", "Map/Lobby/Community_Room",
          "Map/Lobby/Drone_Room", "Doors/Lobby_Drone_Racetrack2", "Map/Lobby/Lobby_Reception", "Map/Lobby/Target_Room1",
          "Map/Lobby/Shooting_Room", "Map/Lobby/LobbyB", "Map/Lobby/Lighting_test_2", "Map/Lobby/Lobby_Corridor"]
TMP = os.environ.get("TEMP", ".")


def dump(asset):
    r = subprocess.run([DOTNET, EXE, GAME + "/Bodycam/Content/Paks", EXTRACTED + "/Bodycam.usmap", os.path.join(TMP, "ue_amb"), "dump", asset], capture_output=True, text=True, encoding="utf8", cwd=os.path.dirname(EXE))
    s = r.stdout
    s = s[s.index("{"):]
    d = json.JSONDecoder(); i = 0; objs = []
    while True:
        while i < len(s) and s[i] in " \r\n": i += 1
        if i >= len(s): break
        try: o, i = d.raw_decode(s, i)
        except Exception: break
        objs.append(o)
    return objs


def prop(o, k, d=None): return o.get("Properties", {}).get(k, d)
def name_of(ref): return ref["ObjectName"].split("'")[-2].split(":")[-1].split(".")[-1] if ref else None
def sound_of(ref): return ref["ObjectPath"].rsplit(".", 1)[0].replace("Bodycam/Content", "/Game") if ref else None


def rot_mat(r):  # UE FRotationMatrix columns
    import math
    p, y, ro = [math.radians(r.get(k, 0)) for k in ("Pitch", "Yaw", "Roll")]
    sp, cp, sy, cy, sr, cr = math.sin(p), math.cos(p), math.sin(y), math.cos(y), math.sin(ro), math.cos(ro)
    return [[cp * cy, sr * sp * cy - cr * sy, -(cr * sp * cy + sr * sy)], [cp * sy, sr * sp * sy + cr * cy, cy * sr - cr * sp * sy], [sp, -sr * cp, cr * cp]]


def world(c, by):
    """Component world position (cm) following AttachParent chains inside the level."""
    loc = prop(c, "RelativeLocation", {"X": 0, "Y": 0, "Z": 0})
    p = [loc["X"], loc["Y"], loc["Z"]]
    par = prop(c, "AttachParent")
    if par:
        pc = by.get(par["ObjectPath"])
        if pc:
            pw, pm = world(pc, by)
            s = prop(pc, "RelativeScale3D", {"X": 1, "Y": 1, "Z": 1})
            m = rot_mat(prop(pc, "RelativeRotation", {}))
            p = [pw[i] + sum(m[i][j] * p[j] * [s["X"], s["Y"], s["Z"]][j] for j in range(3)) for i in range(3)]
            return p, pm
    return p, None


def att_of(a):
    keys = ["bAttenuate", "AttenuationShapeExtents", "FalloffDistance", "DistanceAlgorithm", "AttenuationShape", "bEnableOcclusion",
            "OcclusionLowPassFilterFrequency", "OcclusionVolumeAttenuation", "dBAttenuationAtMax", "bSpatialize", "bEnableReverbSend"]
    return {k: a[k] for k in keys if k in a}


out = []
for lv in LEVELS:
    objs = dump(f"Bodycam/Content/{lv}.umap")
    by = {}
    # exports are addressed as "<umap path>.<index>" in ObjectPath; index = position in export list (1-based after the first file dummy)
    for n, o in enumerate(objs):
        by[o.get("Name")] = o
    byname = {(o.get("Outer", {}).get("ObjectName", "") + "|" + o.get("Name", "")): o for o in objs}
    for o in objs:
        t = o.get("Type")
        if t == "AmbientSound":
            comp = by_name = None
            cname = name_of(prop(o, "AudioComponent"))
            owner = o["Name"]
            cs = [c for c in objs if c.get("Type") == "AudioComponent" and f"PersistentLevel.{owner}'" in c.get("Outer", {}).get("ObjectName", "") + "'"]
            if not cs: continue
            c = cs[0]
            pr = c.get("Properties", {})
            if "Sound" not in pr: continue
            loc = pr.get("RelativeLocation", {"X": 0, "Y": 0, "Z": 0})
            ov = pr.get("AttenuationOverrides")
            out.append({"level": lv, "actor": owner, "label": o.get("ActorLabel"), "sound": sound_of(pr["Sound"]), "vol": pr.get("VolumeMultiplier", 1.0), "pitch": pr.get("PitchMultiplier", 1.0),
                        "loc_cm": [loc["X"], loc["Y"], loc["Z"]], "auto": pr.get("bAutoActivate", True), "override_att": pr.get("bOverrideAttenuation", False),
                        "att_asset": sound_of(pr.get("AttenuationSettings")), "att": att_of(ov) if ov else None, "class": name_of(pr.get("SoundClassOverride")),
                        "spatial": pr.get("bAllowSpatialization", True)})
        elif t == "AudioComponent" and "BP_" in o.get("Outer", {}).get("ObjectName", "") + "x":
            pass
    # BP actors (ceiling fans): audio template on the class
    for o in objs:
        t = o.get("Type", "")
        if t == "BP_CeilingFan_C":
            rc = prop(o, "RootComponent")
            rootc = next((c for c in objs if c.get("Name") == name_of(rc) and f"PersistentLevel.{o['Name']}'" in c.get("Outer", {}).get("ObjectName", "") + "'"), None)
            if rootc is None: continue
            p, _ = world(rootc, {c2["Outer"]["ObjectName"] + "": c2 for c2 in []} or {})
            pr = rootc.get("Properties", {})
            loc = pr.get("RelativeLocation", {"X": 0, "Y": 0, "Z": 0})
            out.append({"level": lv, "actor": o["Name"], "label": "ceiling fan", "sound": "/Game/Ultimate_SFX/Celling_Fan/Celling_fan_Cue", "vol": 1.0, "pitch": 1.0,
                        "loc_cm": [loc["X"], loc["Y"], loc["Z"] - 88.07], "auto": True, "override_att": False, "att_asset": None, "att": None, "class": None, "spatial": True})
os.makedirs(os.path.dirname(OUT), exist_ok=True)
json.dump(out, open(OUT, "w"), indent=1)
print(len(out), "ambients")
for a in out: print(a["level"], a["actor"], a["sound"].split("/")[-1], a["vol"], a["pitch"], [round(x) for x in a["loc_cm"]])
