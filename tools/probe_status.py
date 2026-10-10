"""Fit the survivor status / stress system from BodycamProbe bot-fight recordings.
usage: python -I tools/probe_status.py <probe dir> assets/status_reference.json [rec.csv ...]
Writes the derived constants plus, per recording, the replay inputs (t, speed, aim, shots, damage, tick phase)
and the recorded curves (stress/health/stamina/subtle/strong) that game/src/status.rs replays in a unit test.
Findings (see "findings" in the output): every positive stress step is a whole number of fire_stress(FireRate) shots,
no whiz/damage stress, 1 s tick, -0.1 clamp 0.1..1, stamina +0.1 sprint / -0.05, OutOfBreath/Tremor/HandFatigue stay 0."""
import csv, json, statistics, sys, os

S, A = 'WW_SurvivorStatus_C.', 'AC_BodycamAnimationComponent_C.'
FIGHT = ['rec_1791600252.csv', 'rec_1791600261_respawn.csv', 'rec_1791600269.csv', 'rec_1791600286.csv', 'rec_1791600311.csv']
FIRE_RATES = [0.09, 0.11, 0.13]  # clusters seen: 0.0423 / 0.051 / 0.0597


def fire_stress(fr):  # GA_Fire_Base: MapRangeClamped(FireRate, 0.05..0.8 -> 0.025..0.35)
    return min(max(0.025 + (fr - 0.05) / 0.75 * 0.325, 0.025), 0.35)


def load(path):
    rows = list(csv.reader(open(path)))
    h, out = rows[0], []
    for r in rows[1:]:
        if len(r) != len(h): continue
        d = {}
        for k, v in zip(h, r):
            try: d[k] = float(v)
            except ValueError: d[k] = v
        out.append(d)
    return h, out


def analyse(rows):
    n = len(rows)
    st = [r[S + 'Stress'] for r in rows]; hp = [r[S + 'Health'] for r in rows]; sm = [r[S + 'Stamina'] for r in rows]
    # visible ticks: stamina changed or health +2; fill hidden ones (stamina 0, health full) at 1 s period
    vis = [rows[i]['t'] for i in range(1, n) if sm[i] != sm[i - 1] or hp[i] - hp[i - 1] == 2.0 or st[i] - st[i - 1] <= -0.05]
    ticks = []
    for t in vis:
        while ticks and t - ticks[-1] > 1.5: ticks.append(ticks[-1] + 1.0)
        ticks.append(t)
    while ticks and ticks[0] - rows[0]['t'] > 1.0: ticks.insert(0, ticks[0] - 1.0)
    while ticks and ticks[-1] < rows[-1]['t'] - 1.0: ticks.append(ticks[-1] + 1.0)
    first_tick = ticks[0] if ticks else rows[0]['t'] + 1.0
    shots, frs, dmg, unexplained, tickrows = [0] * n, [0.0] * n, [0.0] * n, [], 0
    for i in range(1, n):
        t0, t1 = rows[i - 1]['t'], rows[i]['t']
        tick = any(t0 < t <= t1 for t in ticks)
        sprint = sm[i] > sm[i - 1]
        exp = st[i - 1]
        if tick and not sprint:
            exp = min(max(exp - 0.1, 0.1), 1.0); tickrows += 1
        res = st[i] - exp
        if hp[i] < hp[i - 1]: dmg[i] = hp[i - 1] - hp[i]
        if abs(res) < 5e-4: continue
        best = min(((abs(res - k * fire_stress(fr)), k, fr) for fr in FIRE_RATES for k in (1, 2, 3)), key=lambda x: x[0])
        if best[0] < 2e-3 and res > 0: shots[i], frs[i] = best[1], best[2]
        elif st[i] >= 1.0 and res < 0: pass  # shot(s) swallowed by the 1.0 clamp at a tick: not recoverable
        else: unexplained.append((round(t1, 3), round(res, 4)))
    return dict(first_tick=first_tick, ticks_hidden=len(ticks) - len(vis), shots=shots, fire_rate=frs, dmg=dmg, unexplained=unexplained)


def main():
    pdir, outp = sys.argv[1], sys.argv[2]
    files = sys.argv[3:] or FIGHT
    ref = dict(recordings={}, findings={})
    all_pos, aim_sub, hand, extra = [], [], 0.0, {}
    for f in files:
        h, rows = load(os.path.join(pdir, f))
        a = analyse(rows)
        hand = max(hand, max(abs(r[A + 'HandFatigue']) for r in rows))
        for c in ('AC_BodycamCamera_C.Tremor', 'AC_BodycamCamera_C.OutOfBreath', 'WW_SurvivorStatus_C.FlashBangEffect'):
            extra[c] = max(extra.get(c, 0), max(abs(r[c]) for r in rows))
        all_pos += [round(s * fire_stress(fr), 4) for s, fr in zip(a['shots'], a['fire_rate']) if s]
        ref['recordings'][f] = dict(
            first_tick=a['first_tick'], unexplained_stress=a['unexplained'], n_shots=sum(a['shots']),
            t=[round(r['t'], 4) for r in rows],
            speed_cm=[round((r['vel_x'] ** 2 + r['vel_y'] ** 2) ** .5, 1) for r in rows],
            aiming=[int(r['ABP_Procedural_Viewmodel_C.IsAiming']) for r in rows],
            shots=a['shots'], fire_rate=a['fire_rate'], damage=a['dmg'],
            stress=[round(r[S + 'Stress'], 5) for r in rows], health=[r[S + 'Health'] for r in rows],
            stamina=[round(r[S + 'Stamina'], 4) for r in rows],
            subtle=[round(r[A + 'StressedSubtleIdleAlpha'], 5) for r in rows],
            strong=[round(r[A + 'StressedStrongIdleAlpha'], 5) for r in rows])
        print(f, 'rows', len(rows), 'shots', sum(a['shots']), 'unexplained', a['unexplained'])
    ref['findings'] = dict(
        fire_stress={str(fr): round(fire_stress(fr), 5) for fr in FIRE_RATES},
        stress_steps_seen=sorted(set(all_pos)),
        whiz_or_damage_stress='none observed (3 hits of 40-80 hp, no jump in Stress)',
        stat_timer_s=1.0, stress_decay=0.1, stress_clamp=[0.1, 1.0], stamina_sprint=0.1, stamina_idle=-0.05, health_regen=2.0,
        subtle='finterp(map(speed 0..100 -> 0.1..1), 10); 0 target aiming', strong='aiming ? map(stress .5..1 -> 0..1) : map(stress 0..1 -> 0..4)',
        max_abs_hand_fatigue=hand, max_abs_other=extra)
    json.dump(ref, open(outp, 'w'), separators=(',', ':'))


main()
