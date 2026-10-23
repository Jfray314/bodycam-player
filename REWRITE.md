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
# VHS movie (death / loading screen background, Movies/Vhs/Vhs_01.bk2, Bink 2): tools/bk2dump (NihAV, see its Cargo.toml)
#   bk2dump Vhs_01.bk2 a.rgb 0 100000 128 72 -> all 5711 RGB frames; pack as luma into a 64x90 grid of 128x72 cells:
#   assets-local/textures/fx/vhs_01_atlas.png (8192x6480 L8). The vertical streak bands from frame ~550 on are the tape's real
#   content (the in-game death / loading screens show them), not a decode error. NihAV: https://nihav.org/nihav.gitbundle cloned to
#   tools/nihav (git-ignored), nihav-rad demuxers/bink.rs magic range widened to KB2n.
cargo run -p game
```
Live values (C++ defaults, real camera motion) come from `tools/ue4ss_probe` (UE4SS Lua mod: F7 property dump,
F8/F9 camera recording), run offline in the real game.

## Controls (from IMC_KBM_CharacterOnFoot)
WASD move, Shift sprint, LeftControl crouch (hold), Space jump, Q/E lean, LMB fire, RMB aim, R tap reload,
R hold 0.25 s mag check, J inspect, 1-5 / wheel swap (Primary, Secondary, Grenade, Melee, Perk), X quick knife attack, G drop item, F pick up, C drone light, F leave drone (items.rs / perks.rs). F2 toggles the camera post-process.

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


## Items (non-gun) assets
`python -I tools/build_items.py [G67,Knife,Flashbang,Kamikaze,DroneController,DroneGoggles]` (ue-export `Grenade/G67`, `Knife`, `Drone*` entries in tools/ue-export/guns.json; writes only guns/<Item>.glb + arms_anims/<Item>.glb, never arms.glb). `tools/weapon_data.py` also bakes `assets/items/<DT_PerkItems row>.json` (BAS anim set + `extra` montages). Item audio cues: extract `Audio/Knife Audio/Grenade FPV_Drone/Sfx`, decode with binka, tools/audio-bake roots.
Mesh-only items (no anim dir of their own, they reuse G67/FLASH9/Knife clips): `ue-export mesh <asset> <Name>` into `models_items/_work/guns/<Name>` + blender `build_glb.py` (FRAME=attach; Folding/Blade with `guns/Knife/anims/*.psa`, same SK_Knife skeleton): RGN (Grenade/RGN/RGO_RGN_Rigged), MB78 (Guns/MB78/MB78), Flashbang02 (9-Bangers), Folding (Melee/Folding/Knife_Folding), Blade (Melee/Blade/SwitchBlade). Karambit: guns.json `Karambit` entry (SKM_Karambit + M_melee_karambit_Argentic, the Basic Bundle icon) -> `build_items.py Karambit`. Sentinel: `Drone.variants` -> built with `build_items.py Kamikaze`, then blender on `guns/Sentinel`.
Drone / RC parts: `tools/attachments.py` bakes `<work>/attachments.json` for Kamikaze / Sentinel / Trataxes / Kyoshin from DT_PerkItems DefaultAttachments + `<Row> Basic Bundle` (skins from DT_PerkSkins or DT_WeaponSkins per shop row); `ueblender.attach_parts` mounts parts on gun sockets or on sockets of parts placed before (RC: chassis Body -> shell FPV_Camera_Support -> carbon FPV_Camera -> camera; antenna Led), rotors/propellers on every Rotor0N bone, RC wheels on every wheel_* bone. RC cars: `python -I tools/build_rccars.py` (chassis + parts + A_FP_<Car>_* clips from /Game/RCCar, outside the plugin AnimRoot). RC + explosion-pack audio: extract `RCCar` / `Explosion_Sounds_Pro_HD_Remake`, dump + binka, audio-bake roots.

## Fast iteration (test map, scripted shots, recordings)
All times are **sim seconds after "viewmodel ready"** (arms + gun meshes spawned; logged as `viewmodel ready at N s real`), so no guessed load offsets. Code: `game/src/dev.rs`, `game/src/testmap.rs`.
| Env | Effect |
|---|---|
| `BODYCAM_MAP=test` | tiny test map: 1 m grid floor (5 m lines darker), stripes + target boards at 5/10/25 m, wall ruler at 30 m, cover, stairs (visual only: flat-ground player), doorway |
| `BODYCAM_MAP=grid` | big test grid (`game/src/grid.rs`, `bots.rs`): lobby with key board, shooting range (static 5/10/25/50 m, sliding + pop-up targets, respawn 4 s), obstacle course (step/jump walls, crate stairs, ramps, stairs, 1 m corridor, doorway), CQB block (rooms, doorways, windows, cover). Real collision: AABB/wedge list `grid::Colliders` (walls, step-up 0.5 m, ramps), also for bots. 6 CQB bots + 2 course guards: patrol, notice (range + cone + line of sight), burst fire with near-miss/whiz/damage (shared `gunfight::BotGun`), die when shot (head 10x), respawn 6 s |
| grid keys | F1 lobby, F3 range, F4 course, F5 CQB (teleport); F7 bots hostile/passive; F10 respawn bots (F2 postfx, F6 gunfight). `BODYCAM_START=range\|course\|cqb` teleports at launch; `BODYCAM_AUTO` also takes F1 F3 F4 F5 F7 F10 |
| `BODYCAM_AUTO="W@0:3,LMB@1:0.5"` | hold key/button at t for d s (keys LMB RMB Ctrl Shift 1-5 R J W A S D Q E Space) |
| `BODYCAM_SHOTS="0.5,1,2"` + `BODYCAM_SCREENSHOT=shots/foo` | several numbered shots `shots/foo_1.png..` in ONE launch, then exit (`BODYCAM_SCREENSHOT=file.png` alone = one shot at `BODYCAM_SCREENSHOT_AT`, default 0.5) |
| `BODYCAM_RECORD=dir:fps:secs` | frame sequence `dir/f_0001.png..` with a fixed sim step of 1/fps (deterministic, runs as fast as it renders) |
| `BODYCAM_SPEED=N` | virtual-time multiplier (shot/auto times stay in sim time; high N gives big dt per frame) |
| `BODYCAM_ASSETS=path` | assets-local override (needed in a worktree; `cargo test` too) |

`tools/shots.sh [scenario...]` (idle walk sprint crouch aim hip_burst ads_burst lean lean_r reload swap swap_back) -> `shots/<scenario>_N.png`, one launch each, on the test map. `GAME=...` picks the exe.
`cargo test -p game` includes headless tests (minimal App, no window), e.g. `player::tests::movement_speeds`.


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
| Blood: flesh hit (Cubit_ImpactFX_Spawner + BloodCloud_Droplets_A + BackSplatter + Blood_Dripping), WW_RagdollSystem bleeding/BloodPool, Screen_Droplets + T_DirtMask_01 lens layers | `blood.rs`, `lens.rs`, `lens.wesl` | BP logic + Cascade modules exact; Niagara internals, puff opacity, puddle growth approximate (see blood.rs header) |
| Blueprint logic (leaning/auto-lean, stamina, blood on camera, abilities) | | in progress (Kismet decompiler) |
| Grenades (GA_Character_Aim_Grenade / GA_ThrowGrenade / Grenade_C / G67_C / Flash_C): RMB pull pin, LMB throw (overhand), toss velocity 1600 cm/s, ray-swept bounce, 3 s fuse, radial damage 200 (350/1000 cm, exp 1.3), flashbang | `items.rs` | real data, physics approximated |
| Knife (GA_Character_QuickAttack, Knife_C.CloseRangeAttack: 50 cm sphere, 125 cm, 100 dmg), drop/pick up (G/F); Karambit / Folding / Blade = same Knife_C with their DT_PerkSkins meshes | `items.rs` | real data |
| Grenade children: n74_C N67 Sticky (sticks, bip, 1.5 s, 200 dmg 250/1000), RGN_C impact (200 dmg 80/500), MB78_C (row G68: 50 dmg 200/2000 then 3 x 20 dmg 100/2000 at 0.2/0.5/1.0 s, fireworks cue), Flash_9-Bangers_C (9 bangs 0.3 s apart, 7.5 dmg + GE_Flashed, PointLight) | `items.rs` | real data |
| Perk slot: Kamikaze FPV drone (A_FP_Drone_Deploy notifies, BP_FPV_Drone Angle-mode controls, DroneMovementComponent 700/3500/1000 cm/s, crash -> GA_ExplodePerk 200 dmg 200/1000 cm exp 2, battery, GA_LeavePerk, cooldown); Sentinel (no Explosive device: crash only self-destroys, Crash Sensibility Angl 100000) | `perks.rs` | real data, see ponytail notes |
| RC cars Trataxes / Kyoshin (BP_RC_Car_Base + ChaosWheeledVehicle setup: torque 100 Nm, MaxRPM 4000, gears 2.4/1.8/1.4/1.2 x 3.6 FWD, wheel radius / friction / travel per BP_*_Wheel, Kyoshin steering curve; W/S/A/D/Space, mouse = FPV look + steer + air control, RMB GA_PerkJump, LMB GA_ExplodePerk, driving-loop cue layers) | `rccar.rs` | real data, raycast-car physics (see ponytail) |
| Tablet LOADOUT EXPLOSIVE/MELEE/GADGET columns | `ui/screens.rs` `Act::Gear` | real items |
| BallisticsVFX impacts/muzzle/shells, death/hit reactions | | todo (item flashlight: GA_Flashlight is the drone's light, see perks.rs) |

## Tablet menu (game/src/ui)
Escape/Tab raises the real tablet (SKM_Tablet + A_FP_Tablet_* clips); bevy_ui screens (HOME/PLAY/LOADOUT/weapon/MENU/SETTINGS/SOCIAL/CREDITS) render to a texture on its screen. Settings -> `%APPDATA%/bodycam-player/settings.json` (BODYCAM_CONFIG overrides). Assets: `assets-local/ui` (Tablet.glb, arms_tablet.glb built with ue-export `all Tablet` + tools/blender/build_glb.py, tex/*.png via ue-export `tex`, Ki fonts). Dev: BODYCAM_AUTO keys Esc Tab F C T Left Right CurR/L/U/D, BODYCAM_CURSOR=x,y, BODYCAM_CLICKS="t:x,y;..." (1920x1080 UI space).

## Attachments / optics (game/src/attach.rs)
Pipeline: `python -I tools/build_attachments.py [jobs]` (tools/attall.py: every DT_WeaponItems attachment whose ItemCompatibilities prefix-matches a gun ItemClassID, one id per DT_WeaponSkins static-mesh skin, material overrides kept; parallel `ue-export att`; blender `build_glb.py` FRAME=attach -> `assets-local/models/attachments/<glb>.glb`), then `python -I tools/attdata.py` bakes `assets/attachments.json` (receiver sockets per gun, item sockets / class / slot / GE_AttachmentStat modifiers / reticle from the optic MI + the gun's default Reticle item, compat + DefaultAttachments per row, baked default-part node names). Runtime (attach.rs): picks replace the default of their slot; parts mount on the socket named after their slot, on the receiver or another part (barrel -> muzzle, optic mount -> optic); with picks, the whole loadout mounts at runtime and the baked default nodes are hidden. Stats: attach::apply_mods (Damage/FireRate/Spread/AmmoCapacity... into Weapon.stats; ReloadTime/SwitchSpeed montage play rates, ADSSpeed, camera + procedural recoil scales into Weapon.mods).
Settings `attachments: {row: [ids]}` (tablet weapon detail: digit keys / buttons; one device per gun socket). In game: C flashlight, Z laser, MMB switch optic (IMC_KBM_CharacterOnFoot). ADS: the optic's AimPoint + AimSightDistance/AimMagnifiedSightDistance behind it replaces the receiver AimPoint. Magnified optic: scope camera (RenderLayers 0 only; arms/gun are layer 2) at the optic Viewpoint, magnification from MI_ScopeOpticReticle (ACOG 3x), drawn as a circle over the eyepiece. Reticles: `python -I tools/reticles.py` bakes `assets/reticles.json` (MI_ReticleData_* texture/colour/scale chain + scope Magnification; the default reticle per optic is a guess) and post-processes the exported masks in `assets-local/models/attachments/reticles` (export with ue-export `tex Bodycam/Content/BodycamScope/Textures/...`, once per texture; delete the png before re-exporting). C/Z toggles play S_WEP_Flashlight_Click_Cue / S_WEP_Laser_Click_Cue; the suppressor's FireLocation socket is the bullet origin (`attach::fire_location`). Mag-check widget (HUD_Magazine, Space=Screen) follows the Magazine bone. Suppressor swaps to Cosmetic.Weapon.SFX.FireSuppressed.
Build note: the shared target's `target/debug/game.exe` is overwritten by every worktree; `tools/_bld.sh` builds and copies game-att.exe right after.

## Multiplayer (game/src/net.rs)
Custom UDP (bincode), host-authoritative damage/kills, 30 Hz state, 100 ms interpolation, reliable event channel. Start: BODYCAM_HOST=7777 / BODYCAM_JOIN=ip[:port] or tablet PLAY > HOST SERVERS. Test: tools/net-test.sh.
Remote visual = placeholder capsule (net::spawn_visual / drive_remotes, RemoteState): switch to character::spawn_character there. Not done: lag compensation, penetration, NAT traversal.
Knife throw (RMB aim, LMB throw, Knife_C: 0.3 s, 15 m/s, aim assist 8 m, 100 dmg, stuck knife = pickup), drone HUD (WBP_DroneInterface Full layout in perks.rs), bots blinded by flashbangs (bots.rs, 18 s x distance falloff). Not ported: bomb perk (GA_TriggerBomb is a game-mode item: needs BombZone/Bombe + C4 export), Acro/Horizon (gamepad-only inputs), underhand grenade (selected by C++ bOverhandGrenade).

`BODYCAM_MODE=bomb` (game/src/bomb.rs): C4 in slot 6 (assets/items/Bomb.json, build_items.py Bomb), zones A/B on `BODYCAM_MAP=test` (x=+-7, z=-4); hold LMB with C4 in a zone 3.49 s to plant, hold F within 2 m 6.46 s to defuse, fuse 60 s (`BODYCAM_BOMBFUSE=s` dev), explosion 1000 dmg 250/2000 cm; round restarts 6 s after the outcome. No net events/beeps (see `// ponytail:` in bomb.rs).

Grenade explosion VFX (`game/src/explosion.rs`): G67_C / Flash_C SpawnEmitterAtLocation Explosion_Frag_small / Explosion_Flash_small at scale 0.5 (Cascade). `tools/uasset` now decodes RawDistributionFloat/Vector (baked lookup tables: op 1 time lookup, op 2 uniform min/max pairs), so spawn counts/lifetimes/sizes/velocities/colour curves come out of the dump; `python -I tools/vfx_export.py <dump.json> assets/vfx/<System>.json` bakes the enabled emitters. Active: Spark burst (M_Flare star, 5 x 0.1 s), Sparks (M_Radial_Gradient, 150 velocity-aligned streaks, drag + gravity), Smoke_expand (Whisps_trans 8x8 flipbook, 10 puffs); the Dust / 2nd Sparks emitters are event-receivers of a disabled collision emitter (never spawn); no particle light (module disabled); camera shake = `effects::Explosion`; G67_C also traces 51 rays (85 deg cone, 50 m) that spawn the bullet-impact FX. Flash_C: 0.1 s PointLight (30000 cd, 1800 K). Textures (assets-local/textures/fx): `ue-export tex Bodycam/Content/BallisticsVFX/Particles/FXMaterials/Flares/Textures/Flare_star` -> flare_star.png, `.../SmokeDust/Textures/whisp_One_512_8x8` -> whisp_one_8x8.png (resized to 1024). Dev: BODYCAM_FX_LIFE=N stretches particle lifetimes.

Flashlight \/ laser devices: `python -I tools\/lights.py` bakes `assets\/lights.json` (Flashlight_C\/Laser_C + *_Child BP specs: cone, intensity, colour, light-function cookie, MuzzleLight; laser MI colour). attach.rs: MainLight = Bevy SpotLight + SpotLightTexture (assets-local\/textures\/light\/<T_Flashlight_Mask_D...>.png via ue-export tex), FLASH_CD_PER_UNIT calibration (ponytail), MuzzleLight on the viewmodel layer; laser = 5 mm beam stretched to the trace + 1 cm dot. Toggles are per gun, only with a device mounted. Dev: BODYCAM_DARK=1 on the grid map (no sun, lobby EV100).
