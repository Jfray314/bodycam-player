"""Dump GE_WeaponStat_* modifiers (cooked unversioned GameplayEffects) to assets/weapon_stats.json.
Layout found by hexdump: FString attribute name ... 0b 01 00 07 <f32 magnitude>."""
import glob, json, re, struct, sys, os
src = sys.argv[1]
out = {}
for fn in sorted(glob.glob(os.path.join(src, 'GE_WeaponStat_*.uexp'))):
    d = open(fn, 'rb').read()
    names = open(fn[:-5] + '.uasset', 'rb').read()
    stats = {}
    for m in re.finditer(rb'[\x05-\x10]\x00\x00\x00(Damage|FireRate|Instability|MagAmount|RagdollForce|[A-Z][A-Za-z]+)\x00', d):
        j = d.find(b'\x0b\x01\x00\x07', m.end())
        if j > 0: stats[m.group(1).decode()] = round(struct.unpack_from('<f', d, j + 4)[0], 4)
    modes = [t.decode() for t in re.findall(rb'Weapon\.FireMode\.(\w+)', names)]
    tags = sorted({t.decode() for t in re.findall(rb'(?:Gameplay|Weapon)\.[\w.]+', names)} - {f'Weapon.FireMode.{m}' for m in modes})
    out[os.path.basename(fn)[14:-5].strip('_')] = {**stats, 'fire_modes': modes, 'tags': tags}
json.dump(out, open('assets/weapon_stats.json', 'w'), indent=1)
for k, v in out.items(): print(k.ljust(14), v)
