# Handoff — Bodycam player 1:1 Rust/Bevy rewrite + multiplayer (2026-10-15, session 8)

## 1. Goal
1:1 Rust (Bevy 0.20) rewrite of UE 5.5.4 game Bodycam's first-person player (movement, camera/lens, guns, viewmodel, attachments,
items, sounds, effects, tablet UI) on the real lobby map, plus multiplayer and a reusable core lib (`bodycam-core`) for other games.
Game install `F:\SteamLibrary\steamapps\common\Bodycam` is READ-ONLY. User wants: keep going through the whole list autonomously,
low usage, builder subagents on `model: sonnet` in worktrees. Done = matches the real game in every state + playable multiplayer.

## 2. Current state (master after bomb merge, `cargo test -p game` 51 pass, `-p bodycam-core` 16 pass, build clean)
Works (merged, screenshot-checked unless noted):
- Lens/post (postfx.wesl; lens-dirt clamped so the skylight no longer stripes), auto exposure (map.rs), real map `scene2` (emissive,
  1024 tex, PhysMat surfaces, Brightness/AlbedoBrightness params), bots A* + accuracy (test map only).
- Guns: 20 rows, reload/bolt reload/mag check/inspect, flinch, GoPro camera adjust (hand visible), muzzle light now WEP Intensity 40
  unitless (~13 lm, 30 ms cap).
- Attachments (attach.rs): 9 glbs, tablet MODIFY, optic ADS, real MI_ReticleData reticles (tools/reticles.py), per-optic magnification
  (ACOG 3x), C flashlight / Z laser / MMB canted, click cues, suppressor sound + FireLocation.
- Items (items.rs, perks.rs, bomb.rs): slots 1-5 (+6 C4 in bomb mode), G drop / F pickup, G67 + flashbang, knife stab X + throw
  (RMB/LMB, no head-follow), Kamikaze FPV drone + real drone HUD, bots blinded by flashbangs, bomb mode `BODYCAM_MODE=bomb`.
- Body: owner FP body (upper body fades out below -40..-65 deg pitch, ponytail), `character.rs` full 3P character
  (`spawn_character(cmd, tf, CharacterState)`) seen from drone/Insert 3P/remote players; `BODYCAM_PUPPET=dx:dz:dyaw` mirror puppet.
- Multiplayer (net.rs): custom UDP + bincode, host-authoritative damage incl. blasts with kill credit, 30 Hz state, interpolation,
  lag-comp rewind (RTT + 100 ms), deathmatch score 40, max 7 players, tablet PLAY > HOST SERVERS (HOST/JOIN/address), MENU scoreboard
  tile. `BODYCAM_HOST=port` / `BODYCAM_JOIN=ip:port`; tests tools/net-test.sh, tools/ui-net-test.sh.
- bodycam-core: weapon_data, sway, recoil, gunfire, ballistics, cues, lens, ffi + include/bodycam_core.h; C example
  bodycam-core/examples/c/main.c (MinGW gcc); parity tests game/src/core_parity.rs; docs/MODDING.md per-engine notes.
Half-done / approximate:
- 3P fire/reload/hit/death are procedural (no 3P montages exported); reload motion barely visible; drone close-up and FP shadow
  after character refactor unverified.
- Workbench top MI_auto_repair_table_01 renders bright: not a material bug — the UE dust layer (Dust Color 0.05, Dust High 0.68)
  isn't ported, and lights/exposure aren't retuned.
- Bomb: no net events, no beep audio, no teams, zones only on test map (lobby has none).
- No underhand grenade (no FP anims), no Acro/Horizon drone modes (gamepad-only), no NAT traversal, no HUD kill feed (game has none).

## 3. Active files
- `game/src/character.rs`, `body.rs`, `bodyik.rs` — 3P character / owner body
- `game/src/net.rs` — multiplayer; `drive_remotes` writes CharacterState
- `game/src/items.rs`, `perks.rs`, `bomb.rs`, `attach.rs`, `weapon.rs`, `viewmodel.rs` (`follow_head`, `no_follow`)
- `game/src/map.rs`, `postfx.wesl`, `effects.rs`, `soft.rs`, `ui/{mod,screens,settings}.rs`
- `tools/ue-export/Scene.cs` (map materials), `tools/blender/ueblender.py`, `tools/bake_tint.py`, `tools/attlib.py`, `tools/reticles.py`
- `bodycam-core/`, `docs/MODDING.md`, `REWRITE.md` (per-system UE provenance)

## 4. Changes made
See `git log` since `6f7ed1a` (wave 1 end): waves 2-3 merges (att-optics, polish, items-inventory, net, FP fixes, core lib,
gap list, 3P character, knife/boards/bomb).

## 5. Failed attempts / gotchas
- Disk C: is nearly full (22 GB free). Worktree builds MUST use the shared target (`CARGO_TARGET_DIR=C:/code/bodycam-player/target`);
  delete `.claude/worktrees/*/target` after merging. Master builds in `target-main` (untracked).
- Shared target's game.exe is overwritten by every builder: build+copy to a per-builder exe name in one command.
- Bevy system fn limit 16 params: group into tuples (done in items.rs knife_input).
- Turning on a camera mid-run panics Bevy directional-light cascades: scope camera parks on a 1 px viewport instead.
- Bevy `Atmosphere` washed out the map — dropped; sky is a gradient cubemap stand-in.
- Spring omega fits (see older history): constant omega 10 is best.
- Builders hit usage limits mid-task; worktrees were sometimes removed — resume via SendMessage, merge master first.

## 6. Specific next steps
1. 3P montages: export third-person fire/reload/hit/death montages (tools/ue-export) and replace the procedural ones in character.rs;
   verify with BODYCAM_PUPPET + BODYCAM_3P; check FP gun shadow and drone close-up.
2. Real two-machine multiplayer test (LAN), then bomb net events + teams, beep audio.
3. Port the UE dust layer (up-facing dust blend) in map loader to fix bright workbench tops; retune if needed.
4. Free disk space (C: 22 GB) before big builds.
