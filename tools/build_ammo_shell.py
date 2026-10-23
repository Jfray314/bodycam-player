import json,os,subprocess,sys
# DT_WeaponSkins "Ammo <x>" static meshes shown on the gun's ammo sockets (viewmodel.rs ammo_items):
# 8 gauge shell (Benelli/Mossberg `shell`), .357 Magnum (Revolver Cartridge_NN bones), .17 HMR (Remington700 bullet bones).
PAKS = r"F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks"
USMAP = r"F:/code/bodycam-player/extracted/Bodycam.usmap"
BLENDER = r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe"
DOTNET = os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe")
H=os.path.dirname(os.path.abspath(__file__))
WORK="F:/code/bodycam-player/assets-local/models/_work_att"
OUT="F:/code/bodycam-player/assets-local/models/attachments"
M="Bodycam/Content/BodycamWeapons/Bullets/Meshes/"
ents=[{"id":"ammo_8_gauge","kind":"Ammo","row":"Ammo Gauge","mesh":M+"SM_8_Gauge__1_","materials":[]},
      {"id":"ammo_357_magnum","kind":"Ammo","row":"Ammo 357Magnum","mesh":M+"SM_357_Magnum","materials":[]},
      {"id":"ammo_17_hmr","kind":"Ammo","row":"Ammo 17HMR","mesh":M+"SM_17_HMR","materials":[]}]
part=os.path.join(WORK,"ammo_todo.json"); json.dump(ents,open(part,"w"))
exe="F:/code/bodycam-player/tools/ue-export/bin/Release/net10.0/ue-export.dll"
r=subprocess.run([DOTNET,exe,PAKS,USMAP,WORK,"att",part],capture_output=True,text=True,cwd=os.path.dirname(exe))
print(r.stdout[-800:],r.stderr[-800:])
for e in ents:
    j=f"{WORK}/att/{e['id']}/{e['id']}.json"; g=f"{OUT}/{e['id']}.glb"
    r=subprocess.run([BLENDER,"--background","--python",H+"/blender/build_glb.py","--",j,g],env={**os.environ,"FRAME":"attach"},capture_output=True,text=True)
    print(os.path.exists(g),(r.stdout+r.stderr)[-300:])
