"""RC cars (DT_PerkItems Trataxes / Kyoshin) -> assets-local/models/{guns,arms_anims}/<Car>.glb.
  python -I tools/attachments.py assets-local/models_items/_work && python -I tools/build_rccars.py [Trataxes,Kyoshin]
Their clips live in /Game/RCCar/Animations (outside the plugin AnimRoot ue-export `all` scans), so chassis + parts + clips are
exported one by one: chassis = DT_PerkSkins '<Car> Chassis', parts = <work>/attachments.json, FP clips = A_FP_<Car>_*."""
import os, subprocess, sys, glob, json
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # repo root
GAME = os.environ.get("BODYCAM_GAME", r"C:\Program Files (x86)\Steam\steamapps\common\Bodycam")  # setup.py also searches Steam libraries
EXTRACTED = os.environ.get("BODYCAM_EXTRACTED", ROOT + "/extracted")
ASSETS = os.environ.get("BODYCAM_ASSETS", ROOT + "/assets-local")
PAKS = GAME + "/Bodycam/Content/Paks"
USMAP = EXTRACTED + "/Bodycam.usmap"
BLENDER = os.environ.get("BLENDER", r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe")
DOTNET = os.environ.get("DOTNET", os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe"))
OUT = ASSETS + "/models"
WORK = os.path.join(os.path.dirname(OUT), "models_items", "_work")
exe = os.path.join(HERE, "ue-export", "bin", "Release", "net10.0", "ue-export.dll")
def run(cmd, env=None, cwd=None):
    r = subprocess.run(cmd, env={**os.environ, **(env or {})}, capture_output=True, text=True, cwd=cwd)
    print("\n".join([l for l in (r.stdout + r.stderr).splitlines() if any(k in l for k in ("WROTE", "Traceback", "Error", "FAIL", "PART MISSING"))][-8:]))
    if r.returncode: sys.exit("failed " + " ".join(cmd)[-200:])

cars = (sys.argv[1] if len(sys.argv) > 1 else "Trataxes,Kyoshin").split(",")
ue = lambda *a: run([DOTNET, exe, PAKS, USMAP, WORK, *a], cwd=os.path.dirname(exe))
att = json.load(open(os.path.join(WORK, "attachments.json")))
CLIPS = EXTRACTED + "/Bodycam/Content/RCCar/Animations"
for car in cars:
    a = att[car]
    ue("mesh", a["receiver"], car)
    os.makedirs(os.path.join(WORK, "guns", car), exist_ok=True)
    for f in glob.glob(os.path.join(WORK, car, "*")):  # `mesh` writes <work>/<name>; the gun layout is <work>/guns/<name>
        os.replace(f, os.path.join(WORK, "guns", car, os.path.basename(f)))
    for p in a["parts"]:
        ue("mesh", p["mesh"], p["row"].replace(" ", "_"))
        d = os.path.join(WORK, "guns", car, "parts", p["row"].replace(" ", "_"))
        os.makedirs(d, exist_ok=True)
        for f in glob.glob(os.path.join(WORK, p["row"].replace(" ", "_"), "*")):
            os.replace(f, os.path.join(d, os.path.basename(f)))
    fp = [f"Bodycam/Content/RCCar/Animations/{car}/Character/{os.path.splitext(os.path.basename(f))[0]}" for f in glob.glob(f"{CLIPS}/{car}/Character/A_FP_*.uasset") if "_Montage" not in f]
    wp = [f"Bodycam/Content/RCCar/Animations/{car}/Car/{os.path.splitext(os.path.basename(f))[0]}" for f in glob.glob(f"{CLIPS}/{car}/Car/A_W_*.uasset") if "_Montage" not in f]
    ue("anim", *fp)
    os.makedirs(os.path.join(WORK, "fp", car), exist_ok=True)
    for f in glob.glob(os.path.join(WORK, "anims", f"A_FP_{car}_*")):
        os.replace(f, os.path.join(WORK, "fp", car, os.path.basename(f)))
    if wp:
        ue("anim", *wp)
        os.makedirs(os.path.join(WORK, "guns", car, "anims"), exist_ok=True)
        for f in glob.glob(os.path.join(WORK, "anims", f"A_W_{car}_*")):
            os.replace(f, os.path.join(WORK, "guns", car, "anims", os.path.basename(f)))
    gj = os.path.join(WORK, "guns", car, car + ".json")
    run([BLENDER, "--background", "--python", os.path.join(HERE, "blender", "build_glb.py"), "--", gj, os.path.join(OUT, "guns", car + ".glb")] + sorted(glob.glob(os.path.join(WORK, "guns", car, "anims", "*.psa"))), {"FRAME": "attach"})
    run([BLENDER, "--background", "--python", os.path.join(HERE, "blender", "build_glb.py"), "--", os.path.join(WORK, "arms", "arms.json"), os.path.join(OUT, "arms_anims", car + ".glb")] + sorted(glob.glob(os.path.join(WORK, "fp", car, "*.psa"))), {"NO_MESH": "1"})
print("done", cars)
