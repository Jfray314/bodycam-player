"""Bake per-weapon data from the dumped cooked JSON (extracted/json) into assets/weapons/<Row>.json.

For each DT_WeaponItems row with an AnimationData set: the BAS_* animation set (viewmodel offsets, ADS, sway
springs, montage lists), PRAS RecoilData, stats GE path, cosmetic sound cues. Asset references to montages are
expanded to {path, length, notifies[{name, time, props}]}; curve assets to their key lists. UE's generated
struct-member suffixes (_12_<GUID>) are stripped from keys.
Usage: python -I tools/weapon_data.py extracted/json assets/weapons
"""
import json, os, re, sys

ROOT, OUT = sys.argv[1], sys.argv[2]
SUFFIX = re.compile(r'_\d+_[0-9A-F]{32}$')
MOUNTS = {'/Game/': 'Content/', '/BodycamAnimationFramework/': 'Plugins/BodycamAnimationFramework/Content/'}
cache = {}

def load(path):
    """'/Game/A/B.B' -> parsed json of Content/A/B.json (or None)."""
    pkg = path.split('.')[0]
    for m, d in MOUNTS.items():
        if pkg.startswith(m):
            f = os.path.join(ROOT, d + pkg[len(m):] + '.json')
            if f not in cache:
                cache[f] = json.load(open(f)) if os.path.exists(f) else None
            return cache[f]
    return None

def export(doc, name):
    return next((e for e in doc['exports'] if e['name'] == name), None) if doc else None

def clean(o):
    if isinstance(o, dict):
        return {SUFFIX.sub('', k): clean(v) for k, v in o.items()}
    if isinstance(o, list):
        return [clean(v) for v in o]
    return o

def curve_keys(p):
    """RichCurve keys as [time, value, interp_mode(0 linear,1 constant,2 cubic), arrive_tangent, leave_tangent]."""
    def keys(c):
        return [[k['time'], k['value'], k['interp_mode'], k['arrive_tangent'], k['leave_tangent']] for k in (c or {}).get('Keys', [])]
    out = {}
    for k, v in p.items():
        if isinstance(v, dict) and 'Keys' in v:
            out[k] = keys(v)
        elif isinstance(v, list) and v and isinstance(v[0], dict) and 'Keys' in v[0]:  # CurveVector FloatCurves[3]
            out[k] = [keys(c) for c in v]
    return out

def expand(v, depth=0):
    if isinstance(v, dict):
        return {k: expand(x, depth) for k, x in v.items()}
    if isinstance(v, list):
        return [expand(x, depth) for x in v]
    if not (isinstance(v, str) and v.startswith('/') and '.' in v) or depth > 2:
        return v
    doc = load(v)
    main = export(doc, v.split('.')[-1])
    if not main:
        return v
    cls, p = main['class'], main.get('properties', {})
    if cls.endswith('AnimMontage') or cls.endswith('AnimSequence'):
        notifies = []
        for n in p.get('Notifies', []):
            obj = export(doc, (n.get('Notify') or '').split('.')[-1])
            notifies.append({'name': n.get('NotifyName'), 'time': round((n.get('LinkValue') or 0) + (n.get('TriggerTimeOffset') or 0), 4),
                             'duration': n.get('duration'), 'props': clean(obj.get('properties', {})) if obj else {}})
        out = {'path': v, 'length': p.get('SequenceLength'), 'rate': p.get('RateScale', 1.0), 'notifies': notifies}
        if cls.endswith('AnimMontage'):  # FAlphaBlend default BlendTime 0.25 when cooked-out
            out['blend_in'] = (p.get('BlendIn') or {}).get('BlendTime', 0.25)
            out['blend_out'] = (p.get('BlendOut') or {}).get('BlendTime', 0.25)
        return out
    if 'Curve' in cls:
        return {'path': v, 'curves': curve_keys(p)}
    if cls.endswith('RecoilData') or cls.endswith('AdditiveMotion_C'):
        return {'path': v, **expand(clean(p), depth + 1)}
    return v

rows = load('/Game/BodycamCore/ItemsDefinition/Gameplay/DT_WeaponItems.DT_WeaponItems')['exports'][0]['properties']['_rows']
os.makedirs(OUT, exist_ok=True)
n = 0
for name, row in rows.items():
    bas_path = row.get('AnimationData')
    if not bas_path:
        continue
    bas = export(load(bas_path), bas_path.split('.')[-1])
    if not bas:
        print('missing BAS', name, bas_path)
        continue
    g = row.get('GameplayDataDefinition', {})
    data = {
        'row': name,
        'item_class': row.get('ItemClassID'),
        'stats_effects': g.get('GameplayEffectsModifier', []),
        'sounds': g.get('CosmeticEffects', {}),
        'default_attachments': [a.get('RowName') for a in row.get('DefaultAttachments', [])],
        'anim_set': expand(clean(bas.get('properties', {}))),
    }
    json.dump(data, open(os.path.join(OUT, name.replace(' ', '_') + '.json'), 'w'), indent=1)
    n += 1
    fire = data['anim_set'].get('Fire') or [{}]
    char = fire[0].get('Character') if isinstance(fire[0], dict) else None
    print(f"{name:14s} reloads={len(data['anim_set'].get('ReloadTacCalm', []))} fire_len={char.get('length') if isinstance(char, dict) else '-'} recoil={'RecoilData' in data['anim_set']}")
print('wrote', n)

# Non-gun items (DT_PerkItems: Knife, Grenade G67/Flash, Kamikaze drone...) -> assets/items/<Row>.json (same anim_set shape)
IT = os.path.join(os.path.dirname(os.path.normpath(OUT)), 'items'); os.makedirs(IT, exist_ok=True)
for name, row in load('/Game/BodycamCore/ItemsDefinition/Gameplay/DT_PerkItems.DT_PerkItems')['exports'][0]['properties']['_rows'].items():
    bas_path = row.get('AnimationData')
    bas = export(load(bas_path), bas_path.split('.')[-1]) if bas_path else None
    if not bas:
        continue
    extra = {}
    if name in ('Grenade G67', 'Grenade G68'):  # clips the BAS does not list (GA_Character_Aim_Grenade cancel, hold pose)
        G = '/BodycamAnimationFramework/Animations/Grenade/G67/Character/A_FP_G67_'
        extra = {k: expand({'Character': G + v + '.A_FP_G67_' + v}) for k, v in (('CancelOverhand', 'Cancel_Overhand'), ('OverhandLoop', 'Overhand_Loop'))}
    if name == 'Kamikaze':  # GA_LaunchPerk leave: A_FP_Drone_Exit_Montage (its DeployDrone(bIsDeploy=false) notify plays the goggles/controller exit clips)
        extra = {'Exit': expand({'Character': '/BodycamAnimationFramework/Animations/Drone/Character/A_FP_Drone_Exit_Montage.A_FP_Drone_Exit_Montage'})}
    json.dump({'extra': extra, 'row': name, 'item_class': row.get('ItemClassID'), 'actor': row.get('ActorClassOverride'),
               'stats_effects': row.get('GameplayDataDefinition', {}).get('GameplayEffectsModifier', []),
               'anim_set': expand(clean(bas.get('properties', {})))}, open(os.path.join(IT, name.replace(' ', '_') + '.json'), 'w'), indent=1)
    print('item', name)

# PRAS fire camera shake (BafCameraShake) with its rotation curve expanded.
shake = export(load('/BodycamAnimationFramework/PRAS/CameraShakes/Shake_Rifle_Standard.Shake_Rifle_Standard'), 'Shake_Rifle_Standard')
json.dump(expand(clean(shake['properties'])), open(os.path.join(os.path.dirname(os.path.normpath(OUT)), 'fire_shake.json'), 'w'), indent=1)
print('wrote fire_shake.json')
