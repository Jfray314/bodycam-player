"""One command: UE paks -> assets-local/models/*.glb + manifest.json.

  python tools/build_models.py [--out C:/code/bodycam-player/assets-local/models] [--only M4A1,Glock17] [--skip-export] [--arms-guns M4A1,Glock17]

1. ue-export (CUE4Parse) reads the paks read-only and writes PSK/PSA/PNG/JSON into <out>/_work
2. Blender (headless, io_scene_psk_psa) builds the .glb files (tools/blender/build_glb.py)
3. manifest.json is written from the produced glbs (tools/blender/inspect_glb.py)
Needs: .NET 10 SDK (see REWRITE.md), Blender 5.2 + io_scene_psk_psa extension, tools/ue-export/build-natives.ps1 run once.
"""
import argparse, json, os, subprocess, sys, glob

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "blender"))
import inspect_glb  # noqa: E402

PAKS = r"F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks"
USMAP = r"C:/code/bodycam-player/extracted/Bodycam.usmap"
BLENDER = r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
DOTNET = os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe")
ATTACH_BONE = "ik_hand_gun"  # UE: BP_EquippableItem.ItemAttachBone = "VB ik_hand_gun" (virtual bone: head -> ik_hand_gun == the bone itself)

ap = argparse.ArgumentParser()
ap.add_argument("--out", default=r"C:/code/bodycam-player/assets-local/models")
ap.add_argument("--only", default="", help="comma list of gun names (default: all guns with FP animations)")
ap.add_argument("--arms-guns", default="M4A1,Glock17", help="guns whose FP clips are baked into arms.glb; others get arms_anims/<Gun>.glb")
ap.add_argument("--skip-export", action="store_true")
ap.add_argument("--resume", action="store_true", help="skip glbs that already exist")
a = ap.parse_args()
out = os.path.abspath(a.out)
work = os.path.join(out, "_work")
only = [g for g in a.only.split(",") if g]
arms_guns = a.arms_guns.split(",")


def run(cmd, env=None, cwd=None):
    print("+", " ".join(cmd)[:200], flush=True)
    r = subprocess.run(cmd, env={**os.environ, **(env or {})}, capture_output=True, text=True, cwd=cwd)
    tail = [l for l in (r.stdout + r.stderr).splitlines() if any(k in l for k in ("WROTE", "Traceback", "Error", "FAIL", "== ", "NO MESH", "skip"))]
    print("\n".join(tail[-12:]))
    if r.returncode or (cmd[0] == BLENDER and "WROTE" not in r.stdout):
        sys.exit("failed: " + " ".join(cmd)[-300:] + chr(10) + r.stdout[-1500:])


def blender(script, args, env=None):
    if a.resume and os.path.exists(args[1]):
        return
    run([BLENDER, "--background", "--python", os.path.join(HERE, "blender", script), "--"] + args, env)


if not a.skip_export:
    exe = os.path.join(HERE, "ue-export", "bin", "Release", "net10.0", "ue-export.dll")
    run([sys.executable, "-I", os.path.join(HERE, "attachments.py"), work])
    run([DOTNET, "build", "-c", "Release", os.path.join(HERE, "ue-export")])
    run([DOTNET, exe, PAKS, USMAP, work, "all"] + ([",".join(["arms"] + only)] if only else []), cwd=os.path.dirname(exe))

psas = lambda d: sorted(glob.glob(os.path.join(work, d, "*.psa")))
guns = sorted(os.path.basename(p) for p in glob.glob(os.path.join(work, "guns", "*")) if os.path.isdir(p))
guns = [g for g in guns if not only or g in only]
os.makedirs(os.path.join(out, "guns"), exist_ok=True)
os.makedirs(os.path.join(out, "arms_anims"), exist_ok=True)

arms_json = os.path.join(work, "arms", "arms.json")
arms_psas = ([p for p in psas("common") if not os.path.basename(p).startswith("A_C_")]  # A_C_* are camera-skeleton clips: tracks go to curves.json
             + [p for g in arms_guns if g in guns for p in psas(f"fp/{g}")])
blender("build_glb.py", [arms_json, os.path.join(out, "arms.glb")] + arms_psas)
# full-body third-person/shadow mesh + ALS clips (ue-export "body" step)
blender("build_glb.py", [os.path.join(work, "body", "body.json"), os.path.join(out, "body.glb")] + psas("body_anims"))
for g in guns:
    blender("build_glb.py", [os.path.join(work, "guns", g, g + ".json"), os.path.join(out, "guns", g + ".glb")] + psas(f"guns/{g}/anims"), {"FRAME": "attach"})
    if g not in arms_guns and psas(f"fp/{g}"):
        blender("build_glb.py", [arms_json, os.path.join(out, "arms_anims", g + ".glb")] + psas(f"fp/{g}"), {"NO_MESH": "1"})

# manifest
import shutil
for f in ("curves.json", "camera_tracks.json"):
    if os.path.exists(os.path.join(work, f)):
        shutil.copy(os.path.join(work, f), os.path.join(out, f))
man = {"units": "metres", "attach_bone": ATTACH_BONE, "files": {}}
def entry(path, kind, **extra):
    i = inspect_glb.info(path)
    return {"kind": kind, "bytes": os.path.getsize(path), "bones": i["bones"], "clips": i["clips"], "textures": i["materials"], **extra}

arms_meta = json.load(open(arms_json))
man["files"]["arms.glb"] = entry(os.path.join(out, "arms.glb"), "arms", frame="viewer (Y-up, forward -Z)", attach_bone=ATTACH_BONE,
                                 sockets=arms_meta["sockets"], material_params={m["material"]: {"scalars": m["scalars"], "colors": m["colors"]} for m in arms_meta["materials"]},
                                 clips_from=arms_guns + ["common (additive/gopro)"])
for g in guns:
    gm = json.load(open(os.path.join(work, "guns", g, g + ".json")))
    man["files"][f"guns/{g}.glb"] = entry(os.path.join(out, "guns", g + ".glb"), "gun", frame="attach: raw UE-bone-local axes; parent scene to joint 'ik_hand_gun' with identity transform",
                                          attach_bone=ATTACH_BONE, skeleton=gm["skeleton"], sockets=gm["sockets"],
                                          material_params={m["material"]: {"scalars": m["scalars"], "colors": m["colors"]} for m in gm["materials"] if "material" in m})
    p = os.path.join(out, "arms_anims", g + ".glb")
    if os.path.exists(p):
        man["files"][f"arms_anims/{g}.glb"] = entry(p, "arms_anims", note="skeleton + A_FP clips only; joint names identical to arms.glb")
json.dump(man, open(os.path.join(out, "manifest.json"), "w"), indent=1)
print("manifest:", os.path.join(out, "manifest.json"), {k: (v["bones"], len(v["clips"]), v["bytes"] // 1000000) for k, v in man["files"].items()})
