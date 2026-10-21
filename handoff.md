# Handoff — Bodycam player 1:1 Rust/Bevy rewrite (2026-10-21, end of session 13)

## 1. Goal
1:1 Rust (Bevy 0.20) rewrite of UE 5.5.4 game Bodycam's first-person player (movement, camera/lens, guns, viewmodel, items,
sounds, effects, UI) on the real lobby map, plus multiplayer and a reusable core lib (`bodycam-core`).
STRICT RULE (user, memory `port-1to1`): everything ported 1:1 from the real game's files; approximate only where the source
can't be read (mark `ponytail:`). Game install `F:\SteamLibrary\steamapps\common\Bodycam` is READ-ONLY. User wants autonomy,
low usage. Never push (no remote).
CURRENT TASK (user chose "Everything"): make the locked gear on the tablet SWAP screens playable 1:1 —
grenades N67 Sticky, RGN Impact, MB78 (row "Grenade G68"), Flash 9-Bangers; melee Karambit, Folding, Blade;
gadgets Sentinel drone, RC cars Trataxes + Kyoshin. Done = each is equippable in SWAP (flip `true` in `GEAR` in
`game/src/ui/screens.rs`) and behaves like its UE BP.

## 2. Current state
master HEAD = `e3f684c`, clean (this handoff aside). `cargo test -p game` 57 pass. Release built (`target/release/game.exe`).
Session 13 shipped (all committed): shot ragdoll impulse; drone dropped not tossed; grenade hold/cancel for all grenades
(ABP_Grenade G67 clips); item idle = BAS IdlePose loop; drone basic bundle (battery, PG-9 warhead, tripwire); tablet 3 cm
further + no gun shake + slow drift; PLAY > CASUAL world cards (relaunch); ALL ~955 real attachments (mount tree, stat mods,
icons); grip-hand IK; Speed attr; remote/3P guns show attachments; tablet menus like the game (loadout card row, weapon view
with live 3D render, MODIFY slot cards, SELECT with category tabs, SWAP with class tabs, locked gear padlocks); full-length
VHS loop; loading screen matched to the user's screenshot.
Playable gear today: Grenade G67, Grenade Flash, Knife, Kamikaze. Everything else in `GEAR` is locked (toast).

## 3. Active files
- `game/src/ui/screens.rs` — `GEAR` table (row, name, icon, playable); swap(), loadout(), weapon(), modify(), select()
- `game/src/ui/showcase.rs` — 3D gun preview (render layer 6, image target)
- `game/src/items.rs` — grenades (grenade_input, projectiles, explode, FLASH_BLAST/G67_BLAST), knife (knife_input,
  knife_flight), `load_item`, `display_name`, SLOT_* consts. ponytail at top lists sticky/RGN/G68 not ported.
- `game/src/perks.rs` — Kamikaze drone (Phase Idle/Deploy/Piloting/Exit); template for Sentinel
- `game/src/attach.rs` — attachments (data(), place(), mount_parts(), apply_mods, grip()), `assets/attachments.json`
- `game/src/viewmodel.rs` — gun_model(), grip_hand IK, base_loop (IdlePose fallback), tablet offset/drift
- `game/src/transition.rs` + `vhs.wesl` — death/loading screens
- `assets/items/*.json` — per-item BAS anim sets (Grenade_N67/RGN/G68/Flash_9-Bangers, Karambit, Folding, Blade, Sentinel exist)
- `tools/build_items.py` (item glbs via ue-export `guns.json` entries + blender), `tools/weapon_data.py` (assets/items json)
- Tools: `target/release/bp2txt.exe extracted/Bodycam.usmap <x.uasset> > extracted/bp/X.txt` (BP decompile),
  ue-export (`tools/ue-export/bin/Release/net10.0/ue-export.dll`; cmds att/tex/mesh/all), `tools/build_attachments.py`,
  `tools/attall.py`, `tools/attdata.py`, `tools/grip_poses.py`, `tools/blender/icon_render.py`, `tools/bk2dump`
- `REWRITE.md` — per-system provenance + export recipes

## 4. Changes made (session 13 commits)
1c7d07f ragdoll impulse; 08473ea drone drop; 2ce363d grenades/idle/drone parts; 3bdc6aa+ddc220b tablet; b0d9b26 PLAY worlds;
810c769 all attachments; 2326bb2 grip IK/speed/3P atts; d3d0239 menus; 5f49d6f VHS; 022d0ed loading; e3f684c icons/locked gear.

## 5. Failed attempts / gotchas
- Parallel ue-export processes collide on shared `_raw` files ("file in use"): re-export failures serially (jobs=1).
- ue-export needs Detex init (done in Program.cs) for BC6H/BC7 textures.
- GripPose socket rotation flips the hand (UE socket axes != imported hand_l axes): use the grip clip's hand_l vs ik_hand_gun.
- VHS "vertical streaks" after frame ~550 are real tape content, not a decode bug. NihAV at tools/nihav (git-ignored, KB2n patch).
- assets/*.json are include_str! at compile time: rebuild release after re-baking or the exe panics on old data.
- Screenshot tests: BODYCAM_AUTO keys, BODYCAM_CLICKS="t:x,y" (UI texture 1920x1080 coords), BODYCAM_SHOTS, BODYCAM_SCREENSHOT,
  BODYCAM_CONFIG=<settings json>; read shots at C:\Users\Administrator\AppData\Local\Temp\gr_N.png.
- Builder subagents were slow; do focused work directly.

## 6. Specific next steps
1. Grenades: decompile BPs (find RGN.uasset, n74.uasset, MB78.uasset, Flash_9-Bangers.uasset under extracted/Bodycam/Content
   -> bp2txt) + GE_GrenadeStat_RGN/N74/G68/Flash_9-Bangers json (damage/radius/fuse). Port: RGN = detonate on impact;
   N67 (n74_C) = stick to the hit surface/actor then fuse; MB78 (G68 row) = its BP behavior; 9-Bangers = repeated bangs.
   Export meshes via build_items.py (add guns.json entries), wire into items.rs, flip GEAR.
2. Melee: Karambit (ActorClassOverride Knife.Knife_C), Folding, Blade — same Knife_C logic, own meshes/anim sets
   (assets/items/Karambit.json etc.); export glbs, make knife_input generic over melee rows, flip GEAR.
3. Sentinel drone (BP_FPV_Drone_Sentinel): decompile, reuse perks.rs flight; port its differences.
4. RC cars Trataxes / Kyoshin (BP_RCCar_*): decompile, new vehicle controller module, meshes, deploy flow, flip GEAR.
5. After each: screenshot-test, `cargo test -p game`, release build, commit.
