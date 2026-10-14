"""Non-gun items (G67 grenade, Flashbang, Knife) -> assets-local/models/{guns,arms_anims}/<Item>.glb without touching arms.glb.
  python -I tools/build_items.py [G67,Knife,Flashbang]   (reuses build_models.py's pipeline pieces; export dir = models_items/_work)"""
import os, subprocess, sys, glob
HERE = os.path.dirname(os.path.abspath(__file__))
PAKS = r"F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks"
USMAP = r"C:/code/bodycam-player/extracted/Bodycam.usmap"
BLENDER = r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
DOTNET = os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe")
OUT = r"C:/code/bodycam-player/assets-local/models"
WORK = os.path.join(os.path.dirname(OUT), "models_items", "_work")
items = (sys.argv[1] if len(sys.argv) > 1 else "G67,Knife,Flashbang").split(",")
exe = os.path.join(HERE, "ue-export", "bin", "Release", "net10.0", "ue-export.dll")
def run(cmd, env=None, cwd=None):
    r = subprocess.run(cmd, env={**os.environ, **(env or {})}, capture_output=True, text=True, cwd=cwd)
    keys = ("WROTE", "Traceback", "Error", "FAIL", "== ", "NO MESH", "skip", "wrote")
    print("\n".join([l for l in (r.stdout + r.stderr).splitlines() if any(k in l for k in keys)][-8:]))
    if r.returncode: sys.exit("failed " + " ".join(cmd)[-200:])
os.makedirs(WORK, exist_ok=True)
run([DOTNET, exe, PAKS, USMAP, WORK, "all", ",".join(["arms"] + items)], cwd=os.path.dirname(exe))
arms_json = os.path.join(WORK, "arms", "arms.json")
for g in items:
    gj = os.path.join(WORK, "guns", g, g + ".json")
    anims = sorted(glob.glob(os.path.join(WORK, "guns", g, "anims", "*.psa")))
    run([BLENDER, "--background", "--python", os.path.join(HERE, "blender", "build_glb.py"), "--", gj, os.path.join(OUT, "guns", g + ".glb")] + anims, {"FRAME": "attach"})
    fp = sorted(glob.glob(os.path.join(WORK, "fp", g, "*.psa")))
    if fp:
        run([BLENDER, "--background", "--python", os.path.join(HERE, "blender", "build_glb.py"), "--", arms_json, os.path.join(OUT, "arms_anims", g + ".glb")] + fp, {"NO_MESH": "1"})
print("done", items)
