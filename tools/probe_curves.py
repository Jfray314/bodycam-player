"""Turn BodycamProbe recordings into camera motion curves for the Rust game (assets/camera_motion.json).

Channels are the final camera view relative to (pawn location, control rotation), minus the idle pose,
converted to Bevy axes: x right, y up, z back (metres); pitch up+, yaw left+, roll ccw+ (degrees).
Loops (walk/sprint/...) are one averaged stride resampled to N phase steps. Events (jump, land) are
averaged windows around detected takeoff/landing. Sequences (fire, reload, lean) are raw at 60 Hz.
Usage: probe_curves.py <rec_with_walk_sprint.csv> <rec_with_rest.csv> <out.json>
"""
import csv, json, math, sys

def load(path):
    out = []
    for r in csv.DictReader(open(path)):
        if None in r.values() or '' in r.values():
            continue
        out.append({k: float(v) for k, v in r.items()})
    return out

def wrap(a):
    return (a + 180) % 360 - 180

def chan(r):
    yaw = math.radians(r['ctl_yaw'])
    dx, dy = r['cam_x'] - r['pawn_x'], r['cam_y'] - r['pawn_y']
    fwd = dx * math.cos(yaw) + dy * math.sin(yaw)
    right = -dx * math.sin(yaw) + dy * math.cos(yaw)
    up = r['cam_z'] - r['pawn_z']
    return [right / 100, up / 100, -fwd / 100,
            wrap(r['cam_pitch'] - r['ctl_pitch']), -wrap(r['cam_yaw'] - r['ctl_yaw']), -r['cam_roll']]

def sane(r):  # drop death-cam / spectator samples
    c = chan(r)
    return abs(c[0]) < 1 and abs(c[2]) < 1 and r['fov'] > 100

def resample(ts, vals, t0, t1, hz=60):
    out, j, n = [], 0, int((t1 - t0) * hz)
    for i in range(n):
        t = t0 + i / hz
        while j + 1 < len(ts) - 1 and ts[j + 1] < t:
            j += 1
        a = (t - ts[j]) / max(ts[j + 1] - ts[j], 1e-6)
        a = min(max(a, 0), 1)
        out.append([vals[j][k] * (1 - a) + vals[j + 1][k] * a for k in range(6)])
    return out

def sub(series, base):
    return [[round(v - b, 5) for v, b in zip(row, base)] for row in series]

def mean(rows):
    return [sum(r[k] for r in rows) / len(rows) for k in range(6)]

def loop(rows, base, steps=64):
    """Average stride: find period by autocorrelation over all channels (normalized), fold, average."""
    ts = [r['t'] for r in rows]
    s = resample(ts, [chan(r) for r in rows], ts[0], ts[-1])
    s = sub(s, base)
    var = [max(1e-9, sum(x[k] ** 2 for x in s) / len(s)) for k in range(6)]
    def ac(lag):
        return sum(sum(s[i][k] * s[i + lag][k] / var[k] for k in range(6)) for i in range(len(s) - lag)) / (len(s) - lag)
    lags = range(int(0.5 * 60), min(int(2.5 * 60), len(s) // 2))
    period = max(lags, key=ac)
    cycles = len(s) // period
    curve = []
    for p in range(steps):
        idx = [c * period + int(p * period / steps) for c in range(cycles)]
        curve.append([round(sum(s[i][k] for i in idx) / len(idx), 5) for k in range(6)])
    return {'period_s': period / 60, 'cycles_averaged': cycles, 'curve': curve}

def window_avg(rows, events, base, pre, post):
    ts = [r['t'] for r in rows]
    vals = [chan(r) for r in rows]
    wins = [resample(ts, vals, t - pre, t + post) for t in events]
    n = min(len(w) for w in wins)
    avg = [[sum(w[i][k] for w in wins) / len(wins) for k in range(6)] for i in range(n)]
    return {'pre_s': pre, 'events_averaged': len(events), 'series': sub(avg, base)}

def seg(rows, m):
    return [r for r in rows if r['marker'] == m and sane(r)]

a, b = load(sys.argv[1]), load(sys.argv[2])
idle = mean([chan(r) for r in seg(b, 4) + seg(b, 0)])          # standing still
crouch_rows = [r for r in seg(b, 1) if r['crouched'] == 1]
crouch_idle = mean([chan(r) for r in crouch_rows if math.hypot(r['vel_x'], r['vel_y']) < 1] or [chan(r) for r in crouch_rows])

def moving(rows, lo):
    return [r for r in rows if math.hypot(r['vel_x'], r['vel_y']) >= lo]

# takeoff/landing detection from the jump segment
jr = seg(b, 2)
takeoffs = [y['t'] for x, y in zip(jr, jr[1:]) if x['falling'] == 0 and y['falling'] == 1]
landings = [y['t'] for x, y in zip(jr, jr[1:]) if x['falling'] == 1 and y['falling'] == 0]

def seq(m):
    rows = seg(b, m)
    ts = [r['t'] for r in rows]
    ctl = [[0, 0, 0, r['ctl_pitch'], -r['ctl_yaw'], 0] for r in rows]
    c0 = ctl[0]
    return {'series': sub(resample(ts, [chan(r) for r in rows], ts[0], ts[-1]), idle),
            'control': [[round(wrap(x[3] - c0[3]), 4), round(wrap(x[4] - c0[4]), 4)] for x in resample(ts, ctl, ts[0], ts[-1])]}

out = {
    'channels': ['x_right_m', 'y_up_m', 'z_back_m', 'pitch_deg', 'yaw_deg', 'roll_deg'],
    'hz': 60,
    'idle': [round(v, 5) for v in idle],
    'crouch_idle': [round(v, 5) for v in crouch_idle],
    'loops': {
        'walk': loop(moving(seg(a, 1), 120), idle),
        'sprint': loop(moving([r for r in a if r['marker'] == 2 and sane(r)], 300), idle),
        'crouch_walk': loop(moving(crouch_rows, 95), crouch_idle),
        'ads_walk': loop(moving(seg(b, 5), 95), idle),
    },
    'events': {
        'jump_takeoff': window_avg(jr, takeoffs, idle, 0.1, 0.5) if takeoffs else None,
        'land': window_avg(jr, landings, idle, 0.1, 1.0) if landings else None,
    },
    'sequences': {name: seq(m) for name, m in
                  [('fire_hip', 6), ('fire_ads', 7), ('reload', 8), ('lean', 9), ('fast_turns', 10)]},
}
json.dump(out, open(sys.argv[3], 'w'))
for k, v in out['loops'].items():
    amp = [round(max(r[i] for r in v['curve']) - min(r[i] for r in v['curve']), 4) for i in range(6)]
    print(f"loop {k:12s} period {v['period_s']:.3f}s cycles {v['cycles_averaged']:2d} p2p {amp}")
for k, v in out['events'].items():
    if v: print(f"event {k:12s} n={v['events_averaged']} len={len(v['series'])}")
print('idle', out['idle'], 'crouch_idle', out['crouch_idle'])
