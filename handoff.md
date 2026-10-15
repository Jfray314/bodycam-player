# Handoff — Bodycam player 1:1 Rust/Bevy rewrite (2026-10-13, session 6)

## 1. Goal
1:1 Rust (Bevy 0.20) rewrite of UE 5.5.4 game Bodycam's first-person player (movement, camera/lens, guns, viewmodel,
recoil/stress gunplay, sounds, effects, UI) on the real lobby/range map. Game install
`F:\SteamLibrary\steamapps\common\Bodycam` is READ-ONLY except the UE4SS mod the user installs. User: "do all the work you can".
Done = matches the real game in every state on every gun.

## 2. Current state (master `ec873d7`, 31 tests pass, `cargo build -p game` clean)
Session 5 commits:
- `b24a372` attach_parts socket fallback (AK `Long_Barrel` -> `Barrel`): AK barrel/front sight were at receiver origin. Akm.glb rebuilt
  (assets-local, gitignored; backup in %TEMP%/audit/Akm.glb.bak). FAL/Vaiga previews checked OK.
- `3451f5b` LowReadySway: gun drops on fast yaw. Decoded from ABP_Procedural_Sway bytecode: FInterpTo(Curve(|LookDeltaInput.X|) *
  (aiming ? AdsScale : Scale), InterpSpeed) from BAS `LowerSway`; ModifyBone VB ik_hand_gun Z (component space = world up). 0.07 cm rms.
- `9685ebc` Look-node AdsScale pins only apply with a magnified optic (node IsAiming = IsAiming && IsScopeCaptureActive) -> removed
  MAGNIFIED / ROT_ADS_PIN; roll sign = +yaw.
- `431c776` Controller recoil Compensation defaults to 1 (no gun sets it; ADS M4A1 burst pulled down by hand: release moves view < 1 deg).
- `ec873d7` Sway refit: ClampMax clamps input*Scale (not input); pivot = live ik_hand_gun (viewmodel::place_arms -> Sway.pivot);
  GAIN_POS 1.8 / GAIN_ROT 1.4; ADS keeps ADS_K_POS 0.5 / ADS_K_ROT 0.75 (AnimNode_Ads is C++, factor measured on M4A1 + Glock17).
- Tooling: `tools/ue-export/Program.cs` ReadScriptData=true; `tools/uasset` bp2txt decompiles ABPs (`target/release/bp2txt.exe
  extracted/Bodycam.usmap <uasset> > extracted/bp/X.txt`); new extracted/bp/ABP_Procedural_Sway.txt, ABP_Procedural_Viewmodel.txt.
- Recording rec_1791908511 (M4A1 ADS sweeps 2-37 s, Glock ADS 39-78 s, M4A1 ADS burst 81-84 s). F8 = start/stop recording (not on
  by default; socket capture is), F9 marker.

Later in session 5:
- `7c68ce8` BAS PoseOffset is mesh component space (fwd = y, right = -x): M4A1 hip gun/hands within ~1 cm of the recording.
- `dc61b77` camadjust.rs: AC_BodycamCamera GoPro adjust (footstep/turn accumulator -> A_FP_GoPro_Adjust01 montage, gate montage+10..100 s).
  Camera motion of the adjust (BafCameraAnimation Camera_Pitch/Yaw/Roll curves, ACL-compressed) not ported: needs a recording.
- Exposure (merged f2ba3db, builder): UE PPV blend resolved (min -7 / max -2 EV100, speeds 13/13, low 40%), camera ev100 4 +
  compensation curve (middle grey), AMBIENT 0.2. Settles in ~1 s; white boards still saturate (materials/lights look right).
- Investigated, not ported (no caller found in exported BPs, probably C++/cues): Bodycam_Player.Flinch (random Glock17 Flinch01/03
  after 0.1-0.15 s), StartBloodOnCamera (ScreenDroplets). High/Low timeline is never played (no bAutoPlay).
- ADS placement: UE aligns the active AimPoint socket (iron sights: receiver 'AimPoint', ZPosition 0) to VB CameraPoint via
  AnimNode_Ads (C++). Real AimPoint in camera frame: M4A1 (-3.2, 1.1, -1.7), Glock (9.3, -2.2, -1.5) cm; ours comes straight from
  AimIdlePose (M4A1 gun 9 cm too close, Glock 9 cm too far). Asked user for an ADS recording of more guns (hip 3 s + ADS 3 s each, F9 between).

## 3. Active files
- `game/src/sway.rs` (run, AxisState::step, low_ready, tests incl. ads_look_sway_matches_recorded_gun_pose), `viewmodel.rs` (place_arms pivot)
- `game/src/player.rs` (CtlRecoil), `weapon.rs`
- `tools/probe_sway.py` (6d writes gun.look_ads per gun), `tools/blender/ueblender.py` (attach_parts)

## 4. Changes made — see `git log b24a372^..ec873d7`

## 5. Failed attempts / gotchas
- Spring omega from stiffness (sqrt(stiff)*speed, 10*sqrt(stiff), 10*stiff) all fit worse than constant omega 10 on hip+ADS.
- DampingSpeed as an input FInterp: worse. SpringSmoothingSpeed as an output FInterp: much worse.
- Exposure blowout seen once at 6 s after reload; did not reproduce in 2 reruns (probably a transient render glitch).
- Fake look key (BODYCAM_AUTO LookR) is 12 counts/frame (~0.7 LookDeltaInput) and sway-only: can't trigger low ready in-game.
- Old gotchas still apply: additive_clips.json is include_str!; game.exe locked while user plays; runs need timeout >= 120 s.

## 6. Specific next steps
1. ADS alignment (see above): when the new recording lands, fit the Ads rule over many guns (scan with the scan.py idea:
   group aimed frames by Ads Speed/AdditiveBlendAlpha; measure gun/AimPoint in the render-camera frame: cam_x/y/z + cam_pitch/yaw/roll).
   Measure ours with BODYCAM_DEBUG_BONES=1 (logs ik_hand_gun/hand_r/hand_l camera-space positions).
2. Per-gun audit of draw/reload/mag check/inspect with BODYCAM_START=<row> screenshots (AK47 done for parts only).
3. Auto exposure takes ~10 s to settle at spawn (speed 3/1 from LobbyB volume; GlobalPostProcess says 13/13 — check which volume wins).
4. Remaining audit: look camera shake, impacts/decals/muzzle smoke (Niagara), shells, full-auto audio, whiz, flashbang, blood, flinch, death.
5. White target boards blow out under the new lights.

## 7. Session 5 end state (usage limit hit)
- Merged to master: exposure (f2ba3db), near-miss crack/whiz audio + GoPro cues in audio bake (d907408; audio.json rebaked, untracked).
  Whiz not yet verified live: run BODYCAM_GUNFIGHT=1 and grep the log for "audio: ...BulletCrack/BulletWizz".
- STOPPED mid-task (check their worktree branches for commits, then merge or redo):
  - per-gun animation audit: branch worktree-agent-ad069c796458502fb
  - shell ejection + FireOut montage: branch worktree-agent-a58e2c864cbce1323 (UE data: Pistol_Eject velocity (0,50..75,100..125) cm/s
    local, size 1.5, life 60 s, spin (0..2,0..2,1..30), gravity -980, collision -> Shell_Pistol_Cue; all guns use DT_Shell row Pistol)
- Waiting on user: ADS recording of more guns (hip 3 s + ADS 3 s each, F9 between, F8 start/stop).

## 8. Session 6 (user away, autonomous) — master 4c0bc21, 34 tests pass
- `7550685`+`9a75b57` ADS = AnimNode_Ads port: gun AimPoint socket (viewmodel::aim_point, (bone, UE cm) -> joint (x,-y,z)*0.01)
  lands on the camera point AND the gun is squared to the view about it (rec_1791925656: 6 guns, <=0.1 cm, identical rotation).
  Glock (slide chain) and M4A1 (pinned) match the recording exactly. Missing: Revolver (rotated Root; rotation target assumes
  UE +Y muzzle). MP9/AK sights sit very close to the eye (receiver AimPoint) - unverified, needs a recording with known guns.
- `8a8030b` Sway: move nodes 0..4 Alpha = Weight*(1-EnableADS) -> strafe sway off while aiming (was shoving ADS gun 10 cm).
- Lens final `f28bede` (user hated d22c384 'tunnel vision'): off-viewport warp samples fade to black past the mid-side
  overshoot (0.075..0.15), jittered 24-tap disk blur capped at r=1 mid-sides, housing fit 1.15 opaque, CA 0.003/2.0, vignette 0.4.
- `52eb551` Bullets: no tracer (WEP.SpawnTrailEffect empty in shipped game); origin = barrel FireLocation (assets/fire_sockets.json).
- Impact/muzzle FX merged (81d6c33): per-surface decals, dust/debris/sparks, muzzle smoke, 1.5-frame muzzle light.
- Merged builders: grid test world + bots (BODYCAM_MAP=grid, F1 lobby F3 range F4 course F5 CQB F7 hostile F10 respawn;
  game/src/grid.rs, bots.rs), map material export fix (tools/ue-export/Scene.cs; assets-local/maps/scene.glb rebuilt, .bak kept),
  per-gun gun_model by row (CZ-P-09/Kobra/M1911 receivers), shell ejection + FireOut (`4c0bc21` fixed invisible shells +
  eject direction: BulletSpawnOrigin frame = right+up).
- Gun sweep script: $TEMP/full.sh (GUNS env) -> shots/full/sheet_<gun>.png; note firing is blocked during the 2.7 s draw.

## 9. Next steps
1. User: ADS recording naming each gun (F9 marker + say the order) to verify AK/MP9/others AimPoints; Revolver aim point.
2. Bots: no pathfinding (stuck on walls when chasing); course ramps/stairs not walked in play.
3. Map: emissive fixtures, 512 px texture cap, sky white, dim walls (re-export ~40 min, tools/ue-export Scene.cs).
4. Shell casing sound (Shell_Pistol_Cue not in audio bake); spin units guessed.
5. Old list: auto exposure settle, look camera shake, impacts/decals/muzzle smoke, flashbang, blood, flinch, death.

## 10. Session 7 (2026-10-14) — master 3f92ccb+
Done & merged: shell drop sound; Revolver ADS (ads_axes); lens = smooth circular per-channel corner mask (postfx.wesl, only corners:
R .935-1, G .915-.985, B .9-.97; BodycamBorders photo plane removed); aimed shots use sight-line pitch (front-sight tip vertex);
FX lit (no white holes/glare); player body (body.rs: DefaultOperator ALS, directional clips, lean, shadow copy, FP arms cast arm/gun
shadow); reload blocks sprint; rapid-fire hand clipping fix; play.bat launcher.
Parallel builders launched (wave 1, worktrees, each copies exe to target/debug/game-<tag>.exe, BODYCAM_ASSETS=main assets-local):
anim (GoPro hand+camera curves, bolt reload, flinch/hit reaction, per-gun audit, kriss_vector row), body (sync blending, turn-in-place,
land/stop, foot IK, legs, 3P IK arms), fx (soft particles, impacts, shells per gun, blood/death, explosion/flashbang, fall damage,
camera shake/exposure, lens), world (map re-export emissive/full tex/sky/PhysMat/collision, bots navmesh+AI+accuracy), ui (tablet
menu: home/play/loadout/weapon/menu/settings/social + HUD), mod (ReShade Bodycam.fx, bodycam-core crate C ABI camera slice, docs/MODDING.md, UE4SS skeleton).
Wave 2 (after wave 1 merges): attachments/optics/laser/flashlight/suppressor, mag-check UI, full-auto loop audio, grenades/flashbang throw,
knife, perks (drone/bomb/cooldowns), helmet flashlight+battery, inventory drop/pickup/weapon wheel. Multiplayer deferred.
Merge note: after merging a builder, `touch game/src/main.rs` before cargo build (stale fingerprints from shared target).
Wave 1 merged (master 6f7ed1a+dfc2134): modding (mods/reshade/Bodycam.fx, bodycam-core crate, docs/MODDING.md, mods/ue4ss skeleton), effects.rs
(blood/death/respawn/fall/explosion/flash; F8 boom F9 flash F11 hit F12 kill), soft.rs soft particles, map scene2 (emissive, 1024 tex, sky,
PhysMat) + bots A*/accuracy, body (synced blend, foot IK, legs, 3P IK arms; Insert = 3P view, BODYCAM_3P), tools/bake_tint.py (operator black),
tablet UI game/src/ui (Esc/Tab; settings %APPDATA%/bodycam-player/settings.json; screen emissive 0.3), anim (GoPro camera curves, bolt reload,
flinch, BODYCAM_START name normalise). Build master in its own dir: CARGO_TARGET_DIR=target-main (shared target is contended).
Wave 2 running: attachments/optics/mag UI/auto audio; grenades/knife/drone/flashlight/inventory; polish (GoPro hand not visible on screen,
Glock/Kobra white, impacts, white boards, lens, tablet placement/text, Land_Heavy).
Wave 2 merged (master 0d9dedd, 42 tests): attach.rs attachments (9 glbs in assets-local/models/attachments, tools/attlib.py attsockets.py,
MODIFY screen, optic ADS, ACOG 4x scope cam, C flashlight / Z laser / MMB canted, mag-count on Magazine bone), suppressor sound swap;
polish (GoPro hand visible: camera pulled 12 cm back during montage; Glock17 glb material fix + Kobra baseColor hand-patched in assets-local
only (redo in pipeline); per-surface impact table; tablet real UMG text/pose/arrow icons/live lens toggle; Land_Heavy); items.rs/perks.rs
(slots 1-5, G drop F pickup, G67/flashbang throw RMB pin LMB throw, knife X, Kamikaze FPV drone; no helmet flashlight in the game).
Open: white boards blowout, lens vs full-frame real refs (need user screenshot), reticle textures, magnification per optic, light/laser
click sounds, suppressor muzzle FX, bomb perk, drone HUD/flight modes, multiplayer, rest of core-lib slices (docs/MODDING.md).
