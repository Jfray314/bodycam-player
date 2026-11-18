"""All attachment meshes -> assets-local/models/attachments/<id>.glb (+ sockets in _work_att/att/<id>/<id>.json).
  python -I tools/build_attachments.py [jobs]   (attall.py -> ue-export att -> blender build_glb.py FRAME=attach, skips built glbs)"""
import os, subprocess, sys, json, zlib
from concurrent.futures import ThreadPoolExecutor
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))  # repo root
GAME = os.environ.get("BODYCAM_GAME", r"C:\Program Files (x86)\Steam\steamapps\common\Bodycam")  # setup.py also searches Steam libraries
EXTRACTED = os.environ.get("BODYCAM_EXTRACTED", ROOT + "/extracted")
ASSETS = os.environ.get("BODYCAM_ASSETS", ROOT + "/assets-local")
PAKS = GAME + "/Bodycam/Content/Paks"
USMAP = EXTRACTED + "/Bodycam.usmap"
BLENDER = os.environ.get("BLENDER", r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe")
DOTNET = os.environ.get("DOTNET", os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe"))
M = ASSETS + "/models"
WORK = os.path.join(M, "_work_att")
OUT = os.path.join(M, "attachments")
jobs = int(sys.argv[1]) if len(sys.argv) > 1 else 6
subprocess.run([sys.executable, "-I", os.path.join(HERE, "attall.py"), WORK], check=True)
lib = json.load(open(os.path.join(WORK, "attall.json")))
import glob
exe = os.path.join(HERE, "ue-export", "bin", "Release", "net10.0", "ue-export.dll")

def incomplete(e):  # exported = json + mesh + every texture it names (parallel ue-export processes can race on shared texture files)
    d = os.path.join(WORK, "att", e["id"])
    j = os.path.join(d, e["id"] + ".json")
    if os.path.exists(os.path.join(OUT, e["id"] + ".glb")):
        return False
    if not os.path.exists(j) or not glob.glob(os.path.join(d, "*.psk*")):
        return True
    return any(not os.path.exists(os.path.join(d, t)) for m in json.load(open(j))["materials"] for t in (m.get("textures") or {}).values())

def export(todo, jobs):  # ue-export is single-threaded: one process per chunk, a mesh always in the same chunk
    def one(k):
        part = os.path.join(WORK, f"attall_todo{k}.json")
        json.dump([e for e in todo if zlib.crc32(e["mesh"].encode()) % jobs == k], open(part, "w"))
        r = subprocess.run([DOTNET, exe, PAKS, USMAP, WORK, "att", part], capture_output=True, text=True, cwd=os.path.dirname(exe))
        return [l for l in (r.stdout + r.stderr).splitlines() if "fail" in l.lower()]
    with ThreadPoolExecutor(jobs) as ex:
        for fails in ex.map(one, range(jobs)):
            print(chr(10).join(fails)[-1500:])

todo = [e for e in lib if incomplete(e)]
if todo:
    export(todo, jobs)
    todo = [e for e in lib if incomplete(e)]
    if todo:  # leftovers (shared-file races): once more, one process
        export(todo, 1)
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
# concurrent Blender starts sometimes miss the psk add-on ("psk.import_file could not be found"): retry the failures one by one
bad = [b for b in (build(e) for e in lib if e["id"] in {x[0] for x in bad}) if b]
print("built", sum(os.path.exists(os.path.join(OUT, e["id"] + ".glb")) for e in lib), "/", len(lib), "failed", len(bad))
for b in bad[:10]:
    print(b)

if bad:
    sys.exit(f"{len(bad)} attachment meshes failed (rerun to retry)")
