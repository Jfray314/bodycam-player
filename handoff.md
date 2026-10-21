# Handoff — Bodycam player 1:1 Rust/Bevy rewrite (2026-10-20, end of session 12)

## 1. Goal
1:1 Rust (Bevy 0.20) rewrite of UE 5.5.4 game Bodycam's first-person player (movement, camera/lens, guns, viewmodel, items,
sounds, effects, UI) on the real lobby map, plus multiplayer and a reusable core lib (`bodycam-core`).
STRICT RULE (user, memory `port-1to1`): everything ported 1:1 from the real game's files. If the real game doesn't have
something, we don't either (even if asked earlier). Approximate only where the source truly can't be read; mark `ponytail:`
and list it here. Only exception kept by the user: T_DirtMask_01 lens scratches overlay.
Game install `F:\SteamLibrary\steamapps\common\Bodycam` is READ-ONLY. User wants autonomy (don't ask until done), low usage,
builders on `model: sonnet` in worktrees with own CARGO_TARGET_DIR (but the user found builders slow; do focused work yourself).
Done = matches the real game in every state + playable multiplayer + portable core/adapters (wasm, mods).

## 2. Current state
master HEAD = `022d0ed`, clean (handoff.md aside). `cargo test -p game` 57 pass, `-p bodycam-core` 16 pass. Release built.
Working (this session): physics ragdoll + 1 s body ride; drone electronics/toss/controller; world blood (decals, spatter,
pools, Blood_Dripping); grenade explosion VFX from Cascade data; fake grenade fixed (body CharGun + FP viewmodel copy hidden on
release); GCN_HitReaction camera shake on damage (no screen blood: real game has none); death "CAM DISCONNECTED" /
"CAM CONNECTED" transition + loading screen (W_ConnectionMenu / W_LoadingMenu exact keys) over the real Vhs_01.bk2 frames;
showcase.bat (demo / play / map).
Stopped builders with uncommitted partial work: match flow (`.claude/worktrees/agent-a5862966f8a3d867e`), real-map bomb
zones/spawns/bots (`.claude/worktrees/agent-a25044b7d67aab522`).

Session 13: shot ragdoll impulse 1:1; drone dropped (no toss); flashbang hold/cancel (ABP_Grenade G67 clips for all grenades);
item idle = BAS IdlePose loop; drone basic bundle (battery + PG-9 warhead/tripwire); tablet 3 cm further out, no gun stress/regrip
layers on it, slow drift (user request); PLAY > CASUAL = world cards (relaunch into lobby/grid/range/course/CQB/test/bodybomb);
ALL real attachments (tools/build_attachments.py + attdata.py, attach.rs mount tree, stat mods, MODIFY slot tabs, grip hand IK).

## 3. Active files
- `game/src/transition.rs` + `vhs.wesl` — death/loading screens; `effects.rs` — death/respawn, hit_shake, flashbang
- `game/src/items.rs` (grenades, hide_thrown), `explosion.rs`, `blood.rs`, `ragdoll.rs`, `perks.rs` (drone), `character.rs`
- `game/src/lens.rs`/`lens.wesl` (dirt only), `postfx.rs`/`postfx.wesl`, `ui/` (tablet UMG rebuild), `hud.rs`
- `game/src/dev.rs` — env hooks (BODYCAM_AUTO keys incl. LookL/R/U/D, Hit, Kill; BODYCAM_SHOTS; BODYCAM_LOADING;
  BODYCAM_EXPLOSIVE; BODYCAM_DUMMY; BODYCAM_MAP=grid; BODYCAM_START=range)
- Tools: `target/release/bp2txt.exe extracted/Bodycam.usmap <x.uasset>` (BP -> txt), `target/release/dump.exe <usmap> <uasset> <outdir>`
  (asset -> json; now decodes MovieSceneFloatChannel), `tools/bk2dump` (Bink 2 frames via NihAV, needs ../nihav checkout),
  `tools/audio-bake`, `tools/vfx_export.py`, `tools/raw_bc.py`
- `REWRITE.md` (per-system provenance + export recipes), `extracted/bp/*.txt`, `extracted/json/...`

## 4. Changes made (session 12)
a7b2414 ragdoll+death cut; f406dff drone; 599adc5/b7ab6da blood; aaf4abe grenade VFX; 5180121+3430da2+b6fb761 fake grenade +
showcase + hit shake; 23ad5d8 lens blood removed; 34d6628 transition/loading; 0b2a201 real VHS frames.

## 5. Failed attempts / gotchas
- Ragdoll NaN = zero-scale FP-hidden bones on the death frame (not colliders/limits).
- Grenade aim = camera ray (UE): on grid range the white panel ahead of spawn bounces it back.
- NihAV Bink 2: KB2n inter frames mis-decode from ~frame 1500 (vertical streaks); frames 0..300 clean. `git://` blocked; use
  https://nihav.org/nihav.gitbundle.
- String tables: JSON export drops them; read the raw `.uexp` FStrings (ST_UI_*.uexp).
- Builder worktrees with no changes get auto-deleted; user may run a stale target/release exe — rebuild release after fixes.
- Bevy 0.20: BorderRadius is a Node field; system param limit 16; bevy_rapier3d 0.37 only physics option.
- Never push (no remote; derived game data is a licensing question).

## 6. Specific next steps (1:1 TODO list, priority order)
0. Session 13 follow-ups: attachment rate/recoil attributes have no base in GE_WeaponStat (1.0 assumed); some attachment
   skins have no ItemIcons (blank SELECT cards); GripPose socket rotation unused (the grip clip's hand_l vs ik_hand_gun is);
   gear SWAP lists only the items items.rs plays (G67, Flash, Knife, Kamikaze); weapon preview camera framing fixed per view.
   Menus: loadout row / weapon view (showcase.rs 3D render) / MODIFY / SELECT / SWAP follow the user's real-game screenshots.
1. Audit for things we have that the real game doesn't (rule!): grep `ponytail:` in game/src, check each against the BPs;
   e.g. drone TOSS_SPEED (UE just drops it — user asked for toss, ask before removing), random ragdoll knock, showcase-only code.
2. Ragdoll impulse: DONE (1c7d07f) for gun shots. Left: corpse shots (GCN_HitReaction picks 1000 when bRagdollOnGround &&
   FullyRagdoll; host targets_at skips dead players), knife/blast pushes (None now), hit bone = nearest ragdoll body to the hit
   point. Headshot: Camera.Transition.Data.InstantKill -> DeactivatePanelInstant (transition.rs supports `instant`),
   DeadHeadShot anim exists. WW_RagdollSystem decompiled to extracted/bp/WW_RagdollSystem.txt.
3. W_ConnectionMenu remaining: decode the ColorTrack (VHS ColorAndOpacity) fully (GreenCurve parse fails in tools/uasset
   props.rs MovieSceneFloatChannel after RedCurve — probably per-channel padding/serialization flag), FastTransition rate 2/4,
   SkipUIAnimation tag; SpectatorHud + WBP_Vhs (REC/BATT/DATE overlay) for spectating after death (round modes).
4. Loading screen details from materials: M_UI_RecIndicator, M_UI_LoadingBar, M_UI_BatteryIndicator are compiled away —
   try the material's exported parameters/expressions in the pak (UE shader bytecode not usable); W_SystemInfoConfigEntry
   ProjectVersion/BuildVersion values from the game's DefaultGame.ini in the pak.
5. In-game HUD audit vs real widgets (UI/Game/Player/*, MenuSystemPro/INGAME): compass (Widget_Compass), warning
   (Widget_Warning), SD card, tablet screens — fonts/positions/colours/animations from the widget json.
6. Sounds audit: death scream exists; check WW_SurvivorStatus Overkill (< -350 HP no scream) + StopScream on respawn;
   GCN_PlayerExplosionNearby / GCN_PlayerFlashed cues (bp files exist) vs effects.rs flashbang/explosion (ear ring 6 m guess).
7. Grenade VFX gaps (aaf4abe report): M_Flare/M_Radial_Gradient/Whisps_trans material looks tuned by eye; smoke flipbook
   linear blend; spark collision; Flash light lumens. Blood gaps (b7ab6da): NS_BloodSquirt_Big / NS_Dripping_Slow / puddle
   Niagara counts (bytecode) are guesses; DustTrail skipped; bleed rate 12/s vs 50/s.
8. Drone: goggles (VB goggles_root virtual bone) not shown; propeller placement approximate; controller pose by eye — use
   the UE hand_l socket + ABP pose.
9. Resume stopped builders (match flow; real-map bomb zones/spawns/bots) or redo directly: review their worktree diffs first.
10. Remaining older items: Workbench top bright (UE dust layer), HUD/loading search in paks (Movies/GameIntro etc. only if
    the game shows them in-match), real two-machine LAN test, NAT traversal, delete old C: copy + `git worktree prune`.
11. Portability (ask the user first): git remote, wasm build of `game`, a concrete mod adapter on bodycam-core.
