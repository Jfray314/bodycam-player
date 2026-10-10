"""Per-shot recoil trajectories from BodycamProbe recordings -> assets/recoil_reference.json.

Usage: probe_recoil.py <probe_dir> <out.json>      (default: extracted/probe, assets/recoil_reference.json)
Shot onset = frame where RecoilAnimationComponent.OutLoc.X (kick) drops > 0.4 cm in one frame.
Groups: gun (Ads Speed 2.3 = M4A1, 3.0 = Glock17) / aim (IsAiming) / kind (single: no shot within 0.3 s
before, auto: follow-up shot of a burst). Per group: mean + std trajectories of OutRot/OutLoc (UE deg/cm)
on a 0.0125 s grid after onset, and the controller (ctl_pitch/ctl_yaw) delta from the pre-onset value
(only shots where the controller did not move in the 2 frames before onset = mouse untouched).
Samples after the next shot's onset are dropped (contaminated), so single-shot means need isolated shots.
"""
import csv, glob, json, os, sys
import numpy as np

P = 'RecoilAnimationComponent.'
CH = ['OutRot.Pitch', 'OutRot.Yaw', 'OutRot.Roll', 'OutLoc.X', 'OutLoc.Y', 'OutLoc.Z']
GUN = {(2.3, 0.3): 'M4A1', (3.0, 1.0): 'Glock17', (2.3, 1.0): 'AK47'}  # (Ads Speed, Ads.AdditiveBlendAlpha); the picked-up rifle fires full auto every 0.11 s = AK47 (FAL, Crossbow are semi)
FILE_GUN = {'rec_1791602986.csv': 'BC5'}  # same (Ads Speed, alpha) as the Glock17; known from the session notes (MP5 hip burst)
DT, N, PRE = 0.0125, 400, 2  # grid step, samples after onset (1.5 s), samples kept before onset


def wrap(a):
    return (a + 180) % 360 - 180


def load(path):
    rows = list(csv.reader(open(path, newline='')))
    cols = {c: i for i, c in enumerate(rows[0])}
    need = ['t', 'ctl_pitch', 'ctl_yaw', 'ABP_Procedural_Viewmodel_C.IsAiming', 'ABP_Procedural_Viewmodel_C.Ads Speed', 'ABP_Procedural_Viewmodel_C.AnimGraphNode_Ads.AdditiveBlendAlpha'] + [P + c for c in CH]
    if any(n not in cols for n in need):
        return None
    good = [r for r in rows[1:] if len(r) == len(rows[0]) and '' not in r]
    ix = [cols[n] for n in need]
    return np.array([[float(r[i]) for i in ix] for r in good])  # t, ctl_p, ctl_y, aiming, ads, 6 channels


def shots(a):
    """Onset = OutLoc.X kick drop (> 0.4 cm/frame) or OutRot.Pitch drop (> 1.5 deg/frame, pistols kick pitch first)."""
    t, dx, dp = a[:, 0], np.diff(a[:, 9]), np.diff(a[:, 6])
    c = [i + 1 for i in range(1, len(dx)) if t[i + 1] - t[i] < 0.05
         and ((dx[i] < -0.4 and dx[i - 1] > -0.4) or (dp[i] < -1.5 and dp[i - 1] > -1.5))]
    out = []
    for i in c:
        if not out or t[i] - t[out[-1]] > 0.04:
            out.append(i)
    return out


def grid(a, i0, k, ang=False):
    t = a[:, 0]
    ts = t[i0] + np.arange(-PRE, N) * DT
    v = np.interp(ts, t, np.unwrap(np.radians(a[:, k])) if ang else a[:, k])
    return np.degrees(v) if ang else v


def main():
    d = sys.argv[1] if len(sys.argv) > 1 else 'extracted/probe'
    out = sys.argv[2] if len(sys.argv) > 2 else 'assets/recoil_reference.json'
    groups, bursts = {}, {}
    for f in sorted(glob.glob(os.path.join(d, 'rec_*.csv'))):
        a = load(f)
        if a is None or len(a) < 100:
            continue
        on = shots(a)
        # bursts: shots < 0.25 s apart; window = first onset .. min(last + 1.2 s, next burst start)
        starts = [0] * bool(on) + [n for n in range(1, len(on)) if a[on[n], 0] - a[on[n - 1], 0] >= 0.25]
        for b, s0 in enumerate(starts):
            s1 = starts[b + 1] if b + 1 < len(starts) else len(on)
            i = on[s0]
            gun = FILE_GUN.get(os.path.basename(f)) or GUN.get((round(a[i, 4], 1), round(a[i, 5], 1)))
            t0 = a[i, 0]
            end = min(a[on[s1 - 1], 0] + 1.2, a[on[s1], 0] - DT if s1 < len(on) else 1e9, a[-1, 0]) - t0
            if gun is None or i < 8 or end < 0.4 or (s0 and t0 - a[on[s0 - 1], 0] < (2.5 if s1 - s0 >= 3 else 0.45)):  # need a settled start (slow components decay ~1.5/s)
                continue
            rec = {'times': [round(float(a[j, 0] - t0), 4) for j in on[s0:s1]], 'len': int(end / DT)}
            for j, c in enumerate(CH):
                rec[c] = grid(a, i, 6 + j)
            cp, cy = grid(a, i, 1, True), grid(a, i, 2, True)
            rec['ctl_pitch'], rec['ctl_yaw'] = cp - cp[0], cy - cy[0]
            rec['quiet'] = bool(abs(rec['ctl_pitch'][:PRE]).max() < 0.05 and abs(rec['ctl_yaw'][:PRE]).max() < 0.05)
            bursts.setdefault(f"{gun}/{'aim' if a[i, 3] > 0.5 else 'hip'}/{s1 - s0}", []).append(rec)
        for n, i in enumerate(on):
            gun = FILE_GUN.get(os.path.basename(f)) or GUN.get((round(a[i, 4], 1), round(a[i, 5], 1)))
            if gun is None or i < 8 or i + N + 2 >= len(a):
                continue
            gap = a[i, 0] - a[on[n - 1], 0] if n else 9
            nxt = a[on[n + 1], 0] - a[i, 0] if n + 1 < len(on) else 9
            key = f"{gun}/{'aim' if a[i, 3] > 0.5 else 'hip'}/{'auto' if gap < 0.3 else 'single'}"
            rec = {'next_gap': nxt}
            for j, c in enumerate(CH):
                rec[c] = grid(a, i, 6 + j)
            cp, cy = grid(a, i, 1, True), grid(a, i, 2, True)
            rec['ctl_pitch'], rec['ctl_yaw'] = cp - cp[0], cy - cy[0]
            rec['quiet'] = bool(abs(rec['ctl_pitch'][:PRE]).max() < 0.05 and abs(rec['ctl_yaw'][:PRE]).max() < 0.05)
            groups.setdefault(key, []).append(rec)
    res = {'dt': DT, 'pre': PRE, 'groups': {}}
    for key, rs in sorted(groups.items()):
        g = {'n': len(rs)}
        for c in CH + ['ctl_pitch', 'ctl_yaw']:
            m, s = [], []
            for j in range(PRE + N):
                v = [r[c][j] for r in rs if (r['quiet'] or not c.startswith('ctl')) and (j - PRE) * DT < r['next_gap'] - DT]
                ok = len(v) >= 3
                m.append(round(float(np.mean(v)), 4) if ok else None)
                s.append(round(float(np.std(v)), 4) if ok else None)
            g[c] = {'mean': m, 'std': s}
        res['groups'][key] = g
        print(key, 'shots', len(rs), 'quiet', sum(r['quiet'] for r in rs))
    # per burst class (gun/aim/shot count): mean+std trajectory from first onset (index PRE) incl. `times` of the shots
    res['bursts'] = {}
    for key, rs in sorted(bursts.items()):
        L = min(r['len'] for r in rs)
        g = {'n': len(rs), 'times': np.mean([r['times'] for r in rs], 0).round(4).tolist(), 'len': L}
        for c in CH + ['ctl_pitch', 'ctl_yaw']:
            v = np.array([r[c][:PRE + L] for r in rs if r['quiet'] or not c.startswith('ctl')])
            g[c] = {'n': len(v), 'mean': v.mean(0).round(4).tolist() if len(v) else [], 'std': v.std(0).round(4).tolist() if len(v) else []}
        res['bursts'][key] = g
        print('burst', key, len(rs), 'len', L)
    json.dump(res, open(out, 'w'))


if __name__ == '__main__':
    main()
