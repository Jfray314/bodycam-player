"""AnimMan_PhysicsAsset (UE json export) -> assets/ragdoll.json for game/src/ragdoll.rs.

UE bone-local (cm, left-handed) -> body.glb bone-local (m): (x, -y, z) * 0.01 (checked against the glb: calf_l in thigh_l
UE (-42.57, 0, 0) cm = glb (-0.4257, 0, 0) m; spine_03 in spine_02 y 0.35 -> -0.0042). Rotations mirror the same way:
quaternion (x, y, z, w) -> (-x, y, -z, w).
Bodies: Sphyl = capsule along its local Z (Length = between the sphere centres), Box = full extents, Sphere.
Constraints: ConstraintBone1 = child (Pos1/PriAxis1/SecAxis1 in its frame), ConstraintBone2 = parent; twist about PriAxis
(X), Swing1 about the third axis (Z), Swing2 about SecAxis (Y); limit 0 + Locked = locked, Free = no limit.
"""
import json, math, sys

Z, X, Y = {'x': 0, 'y': 0, 'z': 0}, {'x': 1, 'y': 0, 'z': 0}, {'x': 0, 'y': 1, 'z': 0}
SRC = 'extracted/json/Content/AdvancedLocomotionV4/CharacterAssets/MannequinSkeleton/PhysicsAssets/AnimMan_PhysicsAsset.json'


def mv(v):
    return [v['x'] * 0.01, -v['y'] * 0.01, v['z'] * 0.01]


def rot_quat(r):  # FRotator::Quaternion
    p, y, ro = (math.radians(r[k]) / 2 for k in ('pitch', 'yaw', 'roll'))
    sp, cp, sy, cy, sr, cr = math.sin(p), math.cos(p), math.sin(y), math.cos(y), math.sin(ro), math.cos(ro)
    q = [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]
    return [-q[0], q[1], -q[2], q[3]]


def axis(v):
    return [v['x'], -v['y'], v['z']]


def main(out='assets/ragdoll.json'):
    ex = json.load(open(SRC))['exports']
    bodies, joints = [], []
    for e in ex:
        p = e['properties']
        if e['struct'] == 'SkeletalBodySetup' and 'BoneName' in p:
            g, shapes = p['AggGeom'], []
            for s in g.get('SphylElems', []):
                shapes.append({'kind': 'capsule', 'center': mv(s['Center']), 'rot': rot_quat(s['Rotation']), 'radius': s['Radius'] * 0.01, 'half_len': s['Length'] * 0.005})
            for s in g.get('BoxElems', []):
                shapes.append({'kind': 'box', 'center': mv(s['Center']), 'rot': rot_quat(s['Rotation']), 'half': [s['X'] * 0.005, s['Y'] * 0.005, s['Z'] * 0.005]})
            for s in g.get('SphereElems', []):
                shapes.append({'kind': 'sphere', 'center': mv(s['Center']), 'rot': [0, 0, 0, 1], 'radius': s['Radius'] * 0.01})
            bodies.append({'bone': p['BoneName'], 'shapes': shapes})
        elif e['struct'] == 'PhysicsConstraintTemplate':
            i = p['DefaultInstance']
            pr = i.get('ProfileInstance', {})
            cone, tw = pr.get('ConeLimit', {}), pr.get('TwistLimit', {})

            def lim(deg, motion):  # None = free, 0 = locked
                m = motion or 'EAngularConstraintMotion::ACM_Limited'
                return None if m.endswith('Free') else 0.0 if m.endswith('Locked') else math.radians(deg if deg is not None else 45.0)
            joints.append({
                'child': i['ConstraintBone1'], 'parent': i['ConstraintBone2'],
                # absent = UE default (zero position, X/Y axes)
                'pos1': mv(i.get('Pos1', Z)), 'pri1': axis(i.get('PriAxis1', X)), 'sec1': axis(i.get('SecAxis1', Y)),
                'pos2': mv(i.get('Pos2', Z)), 'pri2': axis(i.get('PriAxis2', X)), 'sec2': axis(i.get('SecAxis2', Y)),
                'twist': lim(tw.get('TwistLimitDegrees'), tw.get('TwistMotion')),
                'swing1': lim(cone.get('Swing1LimitDegrees'), cone.get('Swing1Motion')),
                'swing2': lim(cone.get('Swing2LimitDegrees'), cone.get('Swing2Motion')),
            })
    json.dump({'source': SRC.split('/')[-1], 'bodies': bodies, 'joints': joints}, open(out, 'w'), indent=1)
    print(len(bodies), 'bodies', len(joints), 'joints ->', out)
    for j in joints:
        print(f"  {j['child']:12s} <- {j['parent']:12s} twist {j['twist']} s1 {j['swing1']} s2 {j['swing2']}")


if __name__ == '__main__':
    main(*sys.argv[1:])
