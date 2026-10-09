# Bodycam player rewrite: tracking

Source: `F:\SteamLibrary\steamapps\common\Bodycam` (UE 5.5, pak v11, Oodle, unversioned cooked properties).
It is read-only: `.claude/settings.json` denies Edit/Write there, and the extractor only opens files for reading.

## Pipeline
```
cargo run --release -p extract -- "F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks" extracted <path substrings...>
python -I tools/ge_stats.py extracted/Bodycam/Content/BodycamCore/AbilitySystem/GameplayEffects/Weapons/Stats
cargo run -p game
```
`extracted/` is git-ignored. Game assets are never committed.

## Blockers
- `Bodycam/Config/*.ini` sits in an AES-encrypted pak. Input bindings come from `IA_*` / `InputConfig_*` assets instead.
- Cooked assets use unversioned properties. Reading arbitrary property values needs a `.usmap` mapping dumped from the
  running game (UE4SS / Dumper-7). Until then, values are pulled by hand-mapped byte patterns (see `tools/ge_stats.py`).
- Character movement and attribute-set C++ live in `Bodycam-Win64-Shipping.exe`, not in assets.

## Source → Rust map
| Original asset(s) | Rust | Status |
|---|---|---|
| `Bodycam_Player` (child of `ALS_Base_CharacterBP`), `PawnData_DefaultCharacter` | `game/src/player.rs` | movement placeholder (ALS default speeds) |
| `AC_BodycamCamera`, `BodycamShake_Walk*/Sprint*/Land*`, `BP_BodycamPlayerCameraManager` | `player.rs::bodycam_shake` | procedural approximation |
| `GE_WeaponStat_*` (WeaponAttributeSet) | `assets/weapon_stats.json` → `weapon.rs` | **real values** |
| `GA_Fire_Base`, `GA_Reload_Base`, `GA_Character_Aim`, `GA_Character_SwapItem`, `GA_Character_Sprint` | `weapon.rs`, `player.rs` | logic approximated |
| `BP_BodycamGameHUD` (in-game part only) | `game/src/hud.rs` | ammo + REC overlay |
| `Bodycam_Player` leaning (IA_Lean, auto-lean by WallDistance, BlockLeft/RightLean), stamina, health, blood-on-camera, walkie-talkie cues, jump/land camera timelines, dead-view camera | | todo (names found in Bodycam_Player) |
| `GA_Character_Crouch`, `GA_Character_HandleFall`, `GA_HitReaction`, `GA_PlayerDeath` | | todo |
| `GA_ThrowGrenade`, `GA_TriggerKnife`, `GA_Character_QuickAttack`, `GA_Flashlight`, attachments (laser/flashlight/optics) | | todo |
| `DT_WeaponItems`, `DT_CommonItems`, `DT_OperatorItems` | | todo |
| `DT_Footstep_Player*`, `Audio/Player`, `Audio/Guns` (sound waves) | | todo: decode SoundWave bulk data |
| `BodycamAnimationFramework` FP montages (equip/fire/reload/inspect), weapon/arm meshes | | todo: needs mesh/anim export |
| `BallisticsVFX` (impacts, muzzle, tracers), `BodycamPostProcessFramework` (lens/VHS look) | | todo |
