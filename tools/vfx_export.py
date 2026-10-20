"""Cascade ParticleSystem (tools/uasset dump json) -> compact runtime json for game/src/explosion.rs.
Usage: python -I tools/vfx_export.py <dump.json> <out.json>
Keeps the enabled emitters (LOD 0) with their distributions resolved: either a baked RawDistribution table
{op,n,stride,sub,ts,tb,v} (op 1 = lookup, op 2 = uniform min/max pairs) or an editor curve {curve:[[t,...],..]}."""
import json, sys

d = json.load(open(sys.argv[1]))['exports']
full = {x['outer'] + '.' + x['name']: x for x in d}


def dist(v):
    if not v:
        return None
    t = v.get('Table')
    if t:
        return {'op': t.get('Op', 0), 'n': t.get('EntryCount', 1), 'stride': t.get('EntryStride', 1), 'sub': t.get('SubEntryStride', 0),
                'ts': t.get('timescale', 0.0), 'tb': t.get('TimeBias', 0.0), 'v': t.get('Values', [])}
    c = full.get(v.get('Distribution', ''))
    if c and 'ConstantCurve' in c['properties']:
        pts = []
        for p in c['properties']['ConstantCurve']['Points']:
            o = p['OutVal']
            pts.append([p['InVal']] + ([o['x'], o['y'], o['z']] if isinstance(o, dict) else [o]))
        return {'curve': pts}
    return None


def mod(m, cls):
    x = full[m]
    return x['properties'] if x['class'].endswith(cls) else None


systems = [x for x in d if x['class'].endswith('ParticleSystem')]
out = {'emitters': []}
for e in systems[0]['properties']['Emitters']:
    em = full[e]
    lod = full[em['properties']['LODLevels'][0]]['properties']
    if not lod.get('bEnabled', True):
        continue
    req = full[lod['RequiredModule']]['properties']
    spawn = full[lod['SpawnModule']]['properties']
    o = {'name': em['properties'].get('EmitterName'), 'material': req['Material'].split('.')[-1],
         'align': req.get('ScreenAlignment', 'PSA_Square').split('::')[-1],
         'sub': [req.get('SubImages_Horizontal', 1), req.get('SubImages_Vertical', 1)],
         'bursts': [[b['Count'], b['Time']] for b in spawn.get('BurstList', [])], 'event_only': False}
    for m in lod['Modules']:
        x = full[m]
        c, p = x['class'].split('.')[-1].replace('ParticleModule', ''), x['properties']
        if p.get('bEnabled') is False:
            continue
        if c == 'EventReceiverSpawn':
            o['event_only'] = True
        elif c == 'Lifetime':
            o['lifetime'] = dist(p['Lifetime'])
        elif c == 'Size':
            o['size'] = dist(p['StartSize'])
        elif c == 'SizeMultiplyLife':
            o['size_life'] = dist(p['LifeMultiplier'])
        elif c == 'Velocity':
            o['velocity'] = dist(p['StartVelocity'])
        elif c == 'ColorOverLife':
            o['color'] = dist(p['ColorOverLife'])
            o['alpha'] = dist(p['AlphaOverLife'])
        elif c == 'Rotation':
            o['rotation'] = dist(p['StartRotation'])
        elif c == 'SubUV':
            o['subuv'] = dist(p['SubImageIndex'])
        elif c == 'AccelerationConstant':
            o['accel_const'] = [p['Acceleration']['x'], p['Acceleration']['y'], p['Acceleration']['z']]
        elif c == 'Acceleration':
            o['accel'] = dist(p['Acceleration'])
        elif c == 'AccelerationDrag':
            o['drag'] = dist(p['DragCoefficientRaw'])
    out['emitters'].append(o)
json.dump(out, open(sys.argv[2], 'w'), separators=(',', ':'))
print(sys.argv[2], len(out['emitters']), 'emitters')
