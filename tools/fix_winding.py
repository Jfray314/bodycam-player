"""Flip triangle winding in a glb, in place, for primitives whose winding mostly disagrees with their vertex normals.
Fixes scene.glb exported before tools/ue-export/Scene.cs dropped its winding swap (map looked inside out).
python -I tools/fix_winding.py assets-local/maps/scene.glb"""
import json, struct, sys
import numpy as np

path = sys.argv[1]
raw = bytearray(open(path, 'rb').read())
n = struct.unpack_from('<I', raw, 12)[0]
j = json.loads(raw[20:20 + n])
bin_off = 20 + n + 8


def view(i, dt, k):
    a = j['accessors'][i]
    bv = j['bufferViews'][a['bufferView']]
    o = bin_off + bv.get('byteOffset', 0) + a.get('byteOffset', 0)
    return o, np.frombuffer(raw, dtype=dt, count=a['count'] * k, offset=o).reshape(-1, k)


flipped = total = 0
for m in j['meshes']:
    for p in m['primitives']:
        at = p['attributes']
        if 'NORMAL' not in at or 'indices' not in p or j['accessors'][p['indices']]['componentType'] != 5125:
            continue
        total += 1
        _, P = view(at['POSITION'], '<f4', 3)
        _, N = view(at['NORMAL'], '<f4', 3)
        o, I = view(p['indices'], '<u4', 1)
        I = I.reshape(-1, 3)
        fn = np.cross(P[I[:, 1]] - P[I[:, 0]], P[I[:, 2]] - P[I[:, 0]])
        d = (fn * (N[I[:, 0]] + N[I[:, 1]] + N[I[:, 2]])).sum(1)
        if (d < 0).sum() > (d > 0).sum():
            raw[o:o + I.nbytes] = I[:, [0, 2, 1]].astype('<u4').tobytes()
            flipped += 1
open(path, 'wb').write(raw)
print(f'flipped {flipped} of {total} primitives')
