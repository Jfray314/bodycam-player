"""Bakes the 'Eject' socket (UE cm, relative to the gun's root bone) of every exported gun into assets/eject_sockets.json.
Guns whose mesh export carries no Eject socket are omitted (UE's GetSocketLocation then returns the component origin)."""
import json, os

G = "C:/code/bodycam-player/assets-local/models/_work/guns/"
out = {}
for d in sorted(os.listdir(G)):
    f = f"{G}{d}/{d}.json"
    if os.path.exists(f):
        for s in json.load(open(f))["sockets"]:
            if s["name"] == "Eject":
                out[d] = [round(v, 4) for v in s["loc"]]
json.dump(out, open(os.path.join(os.path.dirname(__file__), "../assets/eject_sockets.json"), "w"), indent=1)
