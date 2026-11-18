"""The one-off models that build_models / build_items do not cover (recipes in REWRITE.md "Items"):

  python -I tools/build_extras.py            (needs build_items.py's work dir: run it first for Kamikaze / Knife / Tablet)

* mesh-only grenades / knives (RGN, MB78, Flashbang02, Folding, Blade): `ue-export mesh` -> models_items/_work/guns/<Name> -> guns/<Name>.glb
  (Folding / Blade play the Knife clips: same SK_Knife skeleton)
* Sentinel: variant of the Kamikaze anim set, exported by build_items.py Kamikaze, built here
* Tablet: build_items.py Tablet writes guns/Tablet.glb + arms_anims/Tablet.glb, moved to ui/Tablet.glb + ui/arms_tablet.glb
* models/shell.glb: SM_Shell_9mm_Empty (the SpawnShell default mesh) via tools/shell_glb.py
"""
import glob, os, shutil, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # repo root
GAME = os.environ.get("BODYCAM_GAME", r"C:\Program Files (x86)\Steam\steamapps\common\Bodycam")
EXTRACTED = os.environ.get("BODYCAM_EXTRACTED", ROOT + "/extracted")
ASSETS = os.environ.get("BODYCAM_ASSETS", ROOT + "/assets-local")
PAKS = GAME + "/Bodycam/Content/Paks"
BLENDER = os.environ.get("BLENDER", r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe")
DOTNET = os.environ.get("DOTNET", os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe"))
EXE = os.path.join(ROOT, "tools", "ue-export", "bin", "Release", "net10.0", "ue-export.dll")
M = ASSETS + "/models"
W = ASSETS + "/models_items/_work"
C = "Bodycam/Content/BodycamWeapons/"
MESHES = {  # name -> (skeletal mesh asset, clip dir under guns/ to play, or None)
    "RGN": (C + "Grenade/RGN/RGO_RGN_Rigged", None),
    "MB78": (C + "Guns/MB78/MB78", None),
    "Flashbang02": (C + "Grenade/Flash02/Flashbang02", None),
    "Folding": (C + "Melee/Folding/Knife_Folding", "Knife"),
    "Blade": (C + "Melee/Blade/SwitchBlade", "Knife"),
}


def run(cmd, env=None, cwd=None):
    r = subprocess.run(cmd, env={**os.environ, **(env or {})}, capture_output=True, text=True, cwd=cwd)
    if r.returncode:
        sys.exit("failed: " + " ".join(cmd)[:200] + "\n" + (r.stdout + r.stderr)[-800:])
    return r.stdout


def ue(out, *a):
    return run([DOTNET, EXE, PAKS, EXTRACTED + "/Bodycam.usmap", out, *a], cwd=os.path.dirname(EXE))


def glb(gj, out, anims=(), env=None):
    if os.path.exists(out):
        return
    os.makedirs(os.path.dirname(out), exist_ok=True)
    o = run([BLENDER, "--background", "--python", ROOT + "/tools/blender/build_glb.py", "--", gj, out, *anims], {"FRAME": "attach", **(env or {})})
    if "WROTE" not in o:
        sys.exit("blender produced nothing for " + out + "\n" + o[-800:])
    print("  built", out)


os.makedirs(W + "/guns", exist_ok=True)
for name, (asset, clips) in MESHES.items():
    if os.path.exists(f"{M}/guns/{name}.glb"):
        continue
    if not os.path.exists(f"{W}/guns/{name}/{name}.json"):
        ue(W + "/guns", "mesh", asset, name)
    anims = sorted(glob.glob(f"{W}/guns/{clips}/anims/*.psa")) if clips else []
    glb(f"{W}/guns/{name}/{name}.json", f"{M}/guns/{name}.glb", anims)

# Sentinel shares the Kamikaze clips (its work dir is written by `ue-export all Kamikaze`, variants of guns.json "Drone")
sj = f"{W}/guns/Sentinel/Sentinel.json"
if os.path.exists(sj):
    glb(sj, f"{M}/guns/Sentinel.glb", sorted(glob.glob(f"{W}/guns/Sentinel/anims/*.psa")))
else:
    print("  Sentinel work dir missing (run build_items.py Kamikaze)")

# Tablet (ui/)
for src, dst in ((f"{M}/guns/Tablet.glb", f"{ASSETS}/ui/Tablet.glb"), (f"{M}/arms_anims/Tablet.glb", f"{ASSETS}/ui/arms_tablet.glb")):
    if os.path.exists(src):
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.move(src, dst)
        print("  moved", dst)

# models/shell.glb
if not os.path.exists(M + "/shell.glb"):
    tmp = W + "/_shell"
    ue(tmp, "mesh", C + "Guns/Ammunition/SM_Shell_9mm_Empty", "shell")
    ue(tmp + "/shell", "texn", "T_9mm_ALB")
    pskx = glob.glob(tmp + "/shell/*.pskx")[0]
    run([sys.executable, "-I", ROOT + "/tools/shell_glb.py", pskx, tmp + "/shell/T_9mm_ALB.png", M + "/shell.glb"])
    print("  built", M + "/shell.glb")
print("build_extras done")
