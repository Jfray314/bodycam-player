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

## Model/animation export (arms, guns, FP clips -> .glb)
One command (reads the paks read-only, writes only `assets-local/models`, git-ignored):
```
# one-time: .NET 10 SDK in ~/.dotnet-sdk (dotnet-install.ps1 -Channel 10.0), Blender extension, ACL natives
powershell -File tools/ue-export/build-natives.ps1                      # CUE4Parse-Natives.dll (ACL anim decompression), needs VS2022 C++
blender --background --online-mode --command extension install --enable io_scene_psk_psa
python -I tools/build_models.py [--only M4A1,Glock17] [--skip-export]   # ue-export -> PSK/PSA/PNG -> Blender -> glb + manifest.json
blender --background --python tools/blender/verify_glb.py -- assets-local/models/arms.glb   # bones/clips/textures
blender --background --python tools/blender/preview.py -- assets-local/models/_work M4A1 A_FP_M4A1_Idle_Loop01 shots/viewmodel_preview.png
```
Outputs: `arms.glb` (arms mesh + M4A1/Glock17 + common clips), `guns/<Gun>.glb` (gun mesh + `A_W_*` clips),
`arms_anims/<Gun>.glb` (skeleton + FP clips of the other guns; same joint names, load next to arms.glb), `manifest.json`.
Units metres. `arms.glb` is Y-up with forward -Z. Gun glbs are in raw UE-bone-local axes (no Y-up conversion): spawn the gun scene
as a child of the arms joint `ik_hand_gun` (UE `ItemAttachBone` = virtual bone `VB ik_hand_gun`, i.e. the bone itself) with an identity transform.
Clip names equal the UE asset names. Normals are flipped DX->GL; ORM textures are used as-is (R=AO, G=roughness, B=metallic). Textures capped at 2048.
Not exported yet: modular attachment meshes (`BodycamWeapons/Attachement`: barrel/stock/grip/mag/optics; the gun SKM is the receiver only), montages, Mp-FP camera anims.

## Blockers
- `Bodycam/Config/*.ini` sits in an AES-encrypted pak. Input bindings come from `IA_*` / `InputConfig_*` assets instead.
- Cooked assets use unversioned properties. Mappings dumped with UE4SS on 2026-10-09 (engine 5.5.4, includes BP types):
  `extracted/Bodycam.usmap` (v4, uncompressed). Next: a usmap-driven property reader to replace `tools/ge_stats.py`.
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
