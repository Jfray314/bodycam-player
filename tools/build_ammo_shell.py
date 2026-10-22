import json,os,subprocess,sys
PAKS = r"F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks"
USMAP = r"F:/code/bodycam-player/extracted/Bodycam.usmap"
BLENDER = r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
DOTNET = os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe")
H="F:/code/bodycam-player/.claude/worktrees/agent-a1174470e78c52ad2/tools"
WORK="F:/code/bodycam-player/assets-local/models/_work_att"
OUT="F:/code/bodycam-player/assets-local/models/attachments"
ents=[{"id":"ammo_8_gauge","kind":"Ammo","row":"Ammo Gauge","mesh":"Bodycam/Content/BodycamWeapons/Bullets/Meshes/SM_8_Gauge__1_","materials":[]}]
part=os.path.join(WORK,"ammo_todo.json"); json.dump(ents,open(part,"w"))
exe="F:/code/bodycam-player/tools/ue-export/bin/Release/net10.0/ue-export.dll"
r=subprocess.run([DOTNET,exe,PAKS,USMAP,WORK,"att",part],capture_output=True,text=True,cwd=os.path.dirname(exe))
print(r.stdout[-800:],r.stderr[-800:])
for e in ents:
    j=f"{WORK}/att/{e['id']}/{e['id']}.json"; g=f"{OUT}/{e['id']}.glb"
    r=subprocess.run([BLENDER,"--background","--python",H+"/blender/build_glb.py","--",j,g],env={**os.environ,"FRAME":"attach"},capture_output=True,text=True)
    print(os.path.exists(g),(r.stdout+r.stderr)[-300:])
