# Handoff — Bodycam player 1:1 Rust/Bevy rewrite (2026-10-21, end of session 14)

## 1. Goal
1:1 Rust (Bevy 0.20) rewrite of UE 5.5.4 game Bodycam's first-person player (movement, camera/lens, guns, viewmodel, items,
sounds, effects, UI) on the real lobby map, plus multiplayer and a reusable core lib (`bodycam-core`).
STRICT RULE (user, memory `port-1to1`): everything ported 1:1 from the real game's files; approximate only where the source
can't be read (mark `ponytail:`). Game install (`BODYCAM_GAME`) is READ-ONLY. User wants autonomy,
low usage. Public repo: origin = github.com/Jfray314/bodycam-player (local master -> main); push only when the user asks. No game content may ever be committed.
Session 14 task ("port locked gear") is DONE: every SWAP row in `GEAR` (`game/src/ui/screens.rs`) is playable.

## 2. Current state
master HEAD = `c5e736c`, clean (this handoff aside). `cargo test -p game` 59 pass (new: `ui::screens::tests::gear_rows_load`,
`rccar::tests::steering_curve`). Release built (`target/release/game.exe`).
Session 14 commits:
- f37448c grenades: N67 Sticky (n74_C stick + Grenade_ImpactStick + BipGlow + 1.5 s), RGN Impact (explode on hit), MB78 (row G68:
  4 staggered blasts + fireworks cue), Flash 9-Bangers (9 bangs 0.3 s apart, 7.5 dmg + flash + PointLight). `GKind` / `Salvo`
  in items.rs, `BlastSpec.hurt` field, net `Ev::Grenade { row }`.
- f4ba9ce melee Karambit / Folding / Blade (Knife_C logic, own meshes; thrown/stuck/dropped use `viewmodel::gun_model`).
- ab45af1 Sentinel drone (Pulse_Frame + Sentinel Basic Bundle; no Explosive device -> crash self-destroys, no blast; Angl 100000).
- c5e736c RC cars Trataxes / Kyoshin: new `game/src/rccar.rs` (raycast car on the BP_RC_Car_Base Chaos numbers), hooked into
  perks.rs deploy/pilot/exit; `tools/build_rccars.py`; parts chain on part sockets (`ueblender.attach_parts`).

## 3. Active files
- `game/src/rccar.rs` — RC car: spawn, frame (axes from wheel joints), drive, sound layers, FPV camera
- `game/src/perks.rs` — perk flow (Phase Idle/Deploy/Piloting/Exit), drone flight, HUD (uses `perk.speed` / `perk.dist`)
- `game/src/items.rs` — grenades (GKind, salvos, explode), knives, drop/pickup
- `game/src/viewmodel.rs` — `gun_model()` row -> glb, preload `names` list
- `tools/attachments.py`, `tools/blender/ueblender.py` (attach_parts), `tools/build_rccars.py`, `tools/build_items.py`,
  `tools/weapon_data.py` (assets/items json), `tools/audio-bake` (cue roots)
- `REWRITE.md` — recipes for all the new exports (Items section) + source map rows

## 4. Changes made
See commit list above. Assets rebuilt (git-ignored): guns/{RGN,MB78,Flashbang02,Karambit,Folding,Blade,Sentinel,Trataxes,Kyoshin}.glb,
arms_anims/{Karambit,Trataxes,Kyoshin}.glb, audio for RCCar + Explosion_Sounds_Pro_HD_Remake, audio.json rebaked.

## 5. Failed attempts / gotchas
- Bash heredoc with `'EOF'` inside a long `python - <<'EOF'` sometimes fails ("unexpected EOF"): write the script to the scratchpad and run it.
- `MeshRayCast` borrows `Assets<Mesh>`: a system using it can't also take `ResMut<Assets<Mesh>>` (B0002 panic) -> spawn meshes in another system.
- weapon_data.py rewrites assets/items with CRLF noise: `git checkout` files whose diff is empty.
- `extract` filter is a plain substring ("RCCar", not "/RCCar/").
- Screenshot tests: allow ~1 s after a slot key before LMB (draw montage); big glbs (RC 70-80 MB) take a few seconds to show.
- Everything from session 13's gotchas still applies (ue-export serial, Detex, include_str! rebuild, BODYCAM_* dev env vars).

## 6. Specific next steps (nothing pending from the user; pick by value)
1. RC car ponytails (rccar.rs header): WBP_RC_Car_Interface HUD (decompile `extracted/Bodycam/Content/RCCar/UI/WBP_RC_Car_Interface`),
   re-possess a live car instead of destroying it on leave (BP_PerkProxy TryStartProxy), RC flashlight (SpotLight), Kyoshin camera check.
2. Multiplayer: remote players don't see RC cars / Sentinel (net.rs only replicates thrown grenades + blasts).
3. Other open ponytails: `/ponytail-debt` lists them (drone goggles, Acro mode, underhand grenade, SEP_FPV_Drone chain...).
4. After each: screenshot-test, `cargo test -p game`, release build, commit.
