"""Turn the stress layer clips (A_FP_*_Shake_Loop, A_FP_*_Stressed_Additive) into local-space additive deltas, in place.
In UE these are full poses that the Viewmodel ABP blends over the base by StressedSubtle/StrongIdleAlpha; Bevy can only
layer additively, so each bone's track becomes (track - first frame): rotation q * inverse(q0), translation t - t0,
scale and root channels dropped (same conventions as additive_clips.py, which then flags them via the pelvis-at-origin test).
Accessors are shared between clips in these glbs (static tracks), so the deltas are written to NEW accessors appended to
the BIN chunk and the samplers repointed; nothing existing is overwritten. Idempotent (skips tracks already starting at identity/zero).
Run BEFORE tools/additive_clips.py:  python -I tools/stress_additive.py [assets-local/models]
ponytail: reference = the clip's own first frame (the ABP's real reference pose isn't decoded)."""
import json, struct, sys, glob, os


def load(path):
    f = open(path, 'rb'); head = f.read(12)
    n, _ = struct.unpack('<II', f.read(8)); j = json.loads(f.read(n)); rest = bytearray(f.read())
    return head, j, rest


def save(path, head, j, binary):
    js = json.dumps(j, separators=(',', ':')).encode()
    js += b' ' * (-len(js) % 4)
    bin_chunk = struct.pack('<II', len(binary), 0x004E4942) + bytes(binary)
    body = struct.pack('<II', len(js), 0x4E4F534A) + js + bin_chunk
    open(path, 'wb').write(head[:8] + struct.pack('<I', 12 + len(body)) + body)


def qmul(a, b):
    ax, ay, az, aw = a; bx, by, bz, bw = b
    return (aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx,
            aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz)


def convert(path):
    head, j, rest = load(path)
    binary = rest[8:]  # skip the BIN chunk header (length, type)
    names = [nd.get('name') for nd in j['nodes']]
    made, dropped = {}, 0  # source accessor -> delta accessor
    for an in j.get('animations', []):
        if not (an['name'].endswith('Shake_Loop') or an['name'].endswith('Stressed_Additive')):
            continue
        keep = []
        for ch in an['channels']:
            tp = ch['target']['path']
            if tp == 'scale' or names[ch['target']['node']] == 'root':
                dropped += 1
                continue
            keep.append(ch)
            smp = an['samplers'][ch['sampler']]
            src = smp['output']
            if (src, tp) not in made:
                a = j['accessors'][src]; bv = j['bufferViews'][a['bufferView']]
                off = bv.get('byteOffset', 0) + a.get('byteOffset', 0)
                n = 4 if tp == 'rotation' else 3
                vals = [list(struct.unpack_from('<%df' % n, binary, off + i * 4 * n)) for i in range(a['count'])]
                r = vals[0]
                if tp == 'rotation':
                    done = abs(r[3]) > 0.9999999
                    inv = (-r[0], -r[1], -r[2], r[3])
                    out = [q if q[3] >= 0 else tuple(-c for c in q) for q in (qmul(v, inv) for v in vals)]
                else:
                    done = all(abs(c) < 1e-9 for c in r)
                    out = [[v[k] - r[k] for k in range(3)] for v in vals]
                if done:
                    made[(src, tp)] = src
                else:
                    binary += b'\0' * (-len(binary) % 4)
                    j['bufferViews'].append({'buffer': 0, 'byteOffset': len(binary), 'byteLength': 4 * n * len(out)})
                    for v in out:
                        binary += struct.pack('<%df' % n, *v)
                    na = {k: v for k, v in a.items() if k not in ('bufferView', 'byteOffset', 'min', 'max')}
                    na['bufferView'] = len(j['bufferViews']) - 1
                    j['accessors'].append(na)
                    made[(src, tp)] = len(j['accessors']) - 1
            smp['output'] = made[(src, tp)]
        an['channels'] = keep
    if any(k != v for (k, _), v in made.items()) or dropped:
        j['buffers'][0]['byteLength'] = len(binary)
        save(path, head, j, binary)
    return len(made), dropped


root = sys.argv[1] if len(sys.argv) > 1 else 'assets-local/models'
for f in [os.path.join(root, 'arms.glb')] + sorted(glob.glob(os.path.join(root, 'arms_anims', '*.glb'))):
    print(f, convert(f))
