"""All attachment meshes -> assets-local/models/attachments/<id>.glb (+ sockets in _work_att/att/<id>/<id>.json).
  python -I tools/build_attachments.py [jobs]   (attall.py -> ue-export att -> blender build_glb.py FRAME=attach, skips built glbs)"""
import os, subprocess, sys, json
from concurrent.futures import ThreadPoolExecutor
HERE = os.path.dirname(os.path.abspath(__file__))
PAKS = r"F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks"
USMAP = r"F:/code/bodycam-player/extracted/Bodycam.usmap"
BLENDER = r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
DOTNET = os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe")
M = r"F:/code/bodycam-player/assets-local/models"
WORK = os.path.join(M, "_work_att")
OUT = os.path.join(M, "attachments")
jobs = int(sys.argv[1]) if len(sys.argv) > 1 else 6
subprocess.run([sys.executable, "-I", os.path.join(HERE, "attall.py"), WORK], check=True)
lib = json.load(open(os.path.join(WORK, "attall.json")))
todo = [e for e in lib if not os.path.exists(os.path.join(WORK, "att", e["id"], e["id"] + ".json"))]
exe = os.path.join(HERE, "ue-export", "bin", "Release", "net10.0", "ue-export.dll")
def export(k):  # ue-export is single-threaded: one process per chunk
    part = os.path.join(WORK, f"attall_todo{k}.json")
    json.dump(todo[k::jobs], open(part, "w"))
    r = subprocess.run([DOTNET, exe, PAKS, USMAP, WORK, "att", part], capture_output=True, text=True, cwd=os.path.dirname(exe))
    return [l for l in (r.stdout + r.stderr).splitlines() if "fail" in l.lower()]
if todo:
    with ThreadPoolExecutor(jobs) as ex:
        for fails in ex.map(export, range(jobs)):
            print("\n".join(fails)[-1500:])
os.makedirs(OUT, exist_ok=True)

def build(e):
    j, g = os.path.join(WORK, "att", e["id"], e["id"] + ".json"), os.path.join(OUT, e["id"] + ".glb")
    if not os.path.exists(j) or os.path.exists(g):
        return None
    r = subprocess.run([BLENDER, "--background", "--python", os.path.join(HERE, "blender", "build_glb.py"), "--", j, g],
                       env={**os.environ, "FRAME": "attach"}, capture_output=True, text=True)
    return None if os.path.exists(g) else (e["id"], (r.stdout + r.stderr)[-300:])

with ThreadPoolExecutor(jobs) as ex:
    bad = [b for b in ex.map(build, lib) if b]
print("built", sum(os.path.exists(os.path.join(OUT, e["id"] + ".glb")) for e in lib), "/", len(lib), "failed", len(bad))
for b in bad[:10]:
    print(b)
