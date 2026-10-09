# Bodycam player rewrite: tracking

Source: `F:\SteamLibrary\steamapps\common\Bodycam` (UE 5.5.4, pak v11, Oodle, unversioned cooked properties).
Read-only: `.claude/settings.json` denies Edit/Write there; every tool only opens game files for reading.
Game data never goes into git: `extracted/` (raw + JSON dumps) and `assets-local/` (decoded audio, fonts, models)
are ignored. Small derived numeric data that the code needs (`assets/*.json`) is committed.

## Pipeline
```
# 1. pull assets out of the paks (read-only)
cargo run --release -p extract -- "F:/SteamLibrary/steamapps/common/Bodycam/Bodycam/Content/Paks" extracted <path substrings...>
# 2. dump every cooked asset to JSON using the UE4SS mappings (extracted/Bodycam.usmap)
cargo run --release -p uasset --bin dump -- extracted/Bodycam.usmap extracted/Bodycam extracted/json
# 3. bake data the game uses
python -I tools/weapon_data.py extracted/json assets/weapons      # DT_WeaponItems + BAS anim sets + PRAS recoil, fire_shake.json
python -I tools/ge_stats.py extracted/Bodycam/Content/BodycamCore/AbilitySystem/GameplayEffects/Weapons/Stats
python -I tools/probe_curves.py <rec1.csv> <rec2.csv> assets/camera_motion.json   # from the UE4SS probe recordings
# 4. decode audio (BINKA -> wav) into assets-local/audio
cargo run --release -p binka -- extracted/Bodycam/Content/Audio assets-local/audio
# fonts: copy extracted/Bodycam/Content/UI/Fonts/ki/Ki-Bold.ufont -> assets-local/fonts/Ki-Bold.otf
cargo run -p game
```
Live values (C++ defaults, real camera motion) come from `tools/ue4ss_probe` (UE4SS Lua mod: F7 property dump,
F8/F9 camera recording), run offline in the real game.

## Controls (from IMC_KBM_CharacterOnFoot)
WASD move, Shift sprint, LeftControl crouch (hold), Space jump, Q/E lean, LMB fire, RMB aim, R tap reload,
R hold 0.25 s mag check, J inspect, 1-6 / wheel swap. F2 toggles the camera post-process.

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
Default attachments (tools/attachments.py: assets/weapons/<Row>.json -> DT_NewShopItem -> DT_WeaponSkins -> static mesh; socket from ItemClassID) are baked into each gun glb as child nodes of the socket's bone (they follow reload clips). Reticles/ammo skipped. `curves.json` = float anim curves (30 Hz) per clip, `camera_tracks.json` = GoPro A_C_* camera bone keys.
Not exported: montages.


## Source -> Rust map
| Original | Rust | Status |
|---|---|---|
| CharacterMovementComponent (live dump): walk 125, sprint 325, crouch/ADS 100 cm/s, accel 620.9, braking 1000, friction 4, jump 300, air control 0 | `player.rs` | real values |
| Camera motion (AC_BodycamCamera, BodycamShake_*, ALS camera): idle chest mount, walk/sprint/crouch/ADS stride loops, land event, lean pose | `player.rs` + `assets/camera_motion.json` | measured in-game, played back |
| CameraComponent FOV 120 (horizontal, constant incl. ADS) | `postfx.rs::apply_fov` | real |
| M_PP_BodycamMaster (MPC lens distortion 0.33/1.0, CA R/C 3.5 G/M -1.6 B/Y -0.8) + MI_PP_DiskBlur (radius 25 x 0.8, fade exp 10), UE PP defaults (bloom, vignette 0.4, histogram exposure +1, speeds 3/1, motion blur 0.5) | `postfx.rs`, `postfx.wesl` | real params, maths reconstructed (graphs are compiled away) |
| GE_WeaponStat_* (WeaponAttributeSet) | `assets/weapon_stats.json` -> `weapon.rs` | real |
| DT_WeaponItems + BAS_* (montage lengths/notifies: reload, mag check, inspect, draw/holster/first equip) | `assets/weapons/*.json` -> `weapon.rs` | real timings |
| PRAS ControllerRecoil + Shake_Rifle_Standard | `weapon.rs` `Fired`, `player.rs` | real data; Damping recovery not applied |
| HUD: W_PlayerHUD (voice icon only), HUD_Magazine on mag check (window from SetMagCheckUIVisibility notifies, Ki font) | `hud.rs` | screen-space until the viewmodel exists |
| SoundCues, footsteps, breathing, bodycam mic filter (Samsung A52 IR convolution + EQ) | `audio.rs` | in progress |
| Arms/gun meshes + FP animations, BAS viewmodel sway/ADS/offsets, PRAS viewmodel recoil curves | | in progress (mesh export) |
| Blueprint logic (leaning/auto-lean, stamina, blood on camera, abilities) | | in progress (Kismet decompiler) |
| BallisticsVFX impacts/muzzle/shells, flashlight/laser, grenades, knife, drop/take, death/hit reactions | | todo |
