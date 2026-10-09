"""Per-segment stats from a BodycamProbe camera CSV (UE units: cm, deg; X fwd, Y right, Z up)."""
import csv, math, sys
from collections import defaultdict
rows = list(csv.DictReader(open(sys.argv[1])))
seg = defaultdict(list)
for r in rows:
    if None in r.values() or '' in r.values(): continue
    seg[int(r['marker'])].append({k: float(v) for k, v in r.items()})
def wrap(a): return (a + 180) % 360 - 180
def std(xs):
    m = sum(xs) / len(xs); return math.sqrt(sum((x - m) ** 2 for x in xs) / len(xs))
def freq(xs, ts):  # zero crossings of demeaned signal -> Hz
    m = sum(xs) / len(xs); c = sum(1 for a, b in zip(xs, xs[1:]) if (a - m) * (b - m) < 0)
    return c / 2 / max(ts[-1] - ts[0], 1e-6)
for k in sorted(seg):
    s = seg[k]; ts = [r['t'] for r in s]
    sp = [math.hypot(r['vel_x'], r['vel_y']) for r in s]
    # camera offset in pawn yaw frame
    fw, rt, up, dp, dy, dr = [], [], [], [], [], []
    for r in s:
        yaw = math.radians(r['ctl_yaw']); dx, dyy = r['cam_x'] - r['pawn_x'], r['cam_y'] - r['pawn_y']
        fw.append(dx * math.cos(yaw) + dyy * math.sin(yaw)); rt.append(-dx * math.sin(yaw) + dyy * math.cos(yaw))
        up.append(r['cam_z'] - r['pawn_z'])
        dp.append(wrap(r['cam_pitch'] - r['ctl_pitch'])); dy.append(wrap(r['cam_yaw'] - r['ctl_yaw'])); dr.append(r['cam_roll'])
    pz = [r['pawn_z'] for r in s]; vz = [r['vel_z'] for r in s]
    print(f"seg {k:2d} n={len(s):4d} dur={ts[-1]-ts[0]:5.1f}s speed max={max(sp):6.1f} p90={sorted(sp)[int(.9*len(sp))]:6.1f} "
          f"crouch={sum(r['crouched'] for r in s)/len(s):.2f} fall={sum(r['falling'] for r in s)/len(s):.2f} "
          f"fov={min(r['fov'] for r in s):.1f}-{max(r['fov'] for r in s):.1f}")
    print(f"        cam off fwd={sum(fw)/len(fw):6.1f}±{std(fw):4.1f} right={sum(rt)/len(rt):5.1f}±{std(rt):4.1f} up={sum(up)/len(up):6.1f}±{std(up):4.1f} "
          f"| dpitch {sum(dp)/len(dp):5.2f}±{std(dp):4.2f} ({freq(dp,ts):4.2f}Hz) dyaw ±{std(dy):4.2f} ({freq(dy,ts):4.2f}Hz) roll {sum(dr)/len(dr):5.2f}±{std(dr):4.2f} ({freq(dr,ts):4.2f}Hz) "
          f"| z range {max(pz)-min(pz):5.1f} vz max {max(vz):6.1f}")
