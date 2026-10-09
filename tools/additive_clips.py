"""List arm clips that are UE additive deltas (pelvis at the origin instead of ~0.9 m up) -> assets/additive_clips.json.
Played as a full pose they collapse the skeleton; the viewmodel layers them additively instead.
Also strips, in place, their scale channels (Bevy adds additive scale: 1 + 1 = 2 per bone; UE additives carry none)
and their root channels (root holds the export's Z-up -> Y-up rotation, which must not be added twice).
Run after every model rebuild (tools/build_models.py)."""
import json, struct, sys, glob, os

def load(path):
    f = open(path, 'rb'); head = f.read(12)
    n, _ = struct.unpack('<II', f.read(8)); j = json.loads(f.read(n)); rest = f.read()
    return head, j, rest

def save(path, head, j, rest):
    js = json.dumps(j, separators=(',', ':')).encode()
    js += b' ' * (-len(js) % 4)
    body = struct.pack('<II', len(js), 0x4E4F534A) + js + rest
    open(path, 'wb').write(head[:8] + struct.pack('<I', 12 + len(body)) + body)

def strip_scale(path, additive):
    head, j, rest = load(path)
    names = [nd.get('name') for nd in j['nodes']]
    hit = 0
    for an in j.get('animations', []):
        if an['name'] in additive:
            keep = [c for c in an['channels'] if c['target']['path'] != 'scale' and names[c['target']['node']] != 'root']
            hit += len(an['channels']) - len(keep)
            an['channels'] = keep
    if hit:
        save(path, head, j, rest)
    return hit

def clips(path):
    _, j, rest = load(path)
    binary = rest[8:]
    names = [nd.get('name') for nd in j['nodes']]
    for an in j.get('animations', []):
        for ch in an['channels']:
            if names[ch['target']['node']] == 'pelvis' and ch['target']['path'] == 'translation':
                a = j['accessors'][an['samplers'][ch['sampler']]['output']]
                bv = j['bufferViews'][a['bufferView']]
                x, y, z = struct.unpack_from('<3f', binary, bv.get('byteOffset', 0) + a.get('byteOffset', 0))
                yield an['name'], abs(x) + abs(y) + abs(z) < 1e-3

root = sys.argv[1] if len(sys.argv) > 1 else 'assets-local/models'
files = [os.path.join(root, 'arms.glb')] + sorted(glob.glob(os.path.join(root, 'arms_anims', '*.glb')))
out = sorted({n for f in files for n, add in clips(f) if add})
json.dump(out, open('assets/additive_clips.json', 'w'), indent=0)
print(len(out), 'additive:', out)
print('scale channels stripped:', sum(strip_scale(f, set(out)) for f in files))
