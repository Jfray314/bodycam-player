# Putting the Bodycam player into other games

## Architecture

```
 host game  <->  engine adapter (per engine)  <->  bodycam-core (this repo, C ABI, no engine deps)
```

- `bodycam-core` (crate `bodycam-core`, builds `bodycam_core.dll/.so`, header `bodycam-core/include/bodycam_core.h`) holds
  the engine-independent maths: the Rust port of the Bodycam camera motion. Each frame the adapter feeds it input and gets
  camera offsets back. It never touches the engine, files, or the network.
- Adapters are small and engine specific:
  - Unreal games: UE4SS C++ (or Lua) mod. Example skeleton: `mods/ue4ss/BodycamCam/`.
  - Unity games: BepInEx (Mono) or MelonLoader plugin, P/Invoke into the dll.
  - Others (Source, id Tech, custom): a native plugin or injected DLL that calls the same C functions.
- The ReShade shader (`mods/reshade/Bodycam.fx`) is engine independent and covers the "camera look" with no adapter at all.

### Conventions of the core
Unreal convention on the public side: metres, x right, y up, z back; degrees; pitch up+, yaw right+, roll clockwise+.
Output offsets are relative to the character's feet (position) and to the control rotation (rotation). Adapters convert to the
host's axes and units (Unreal is cm: multiply positions by 100; Unity is metres, left-handed with z forward: negate z and flip signs
of yaw and roll as needed).

## What each host must provide

| Need | Why |
|---|---|
| Camera override | Apply `pos`/`rot` to the host camera (UE: `PlayerCameraManager` view target POV; Unity: camera transform after the host's update) |
| Hide host viewmodel | The host's own arms/gun must be hidden, or they fight ours |
| Spawn our meshes | Arms and gun viewmodel (a later slice) come from extracted assets, spawned and attached to the camera |
| Raw input | Mouse delta in degrees (apply the host's sensitivity yourself), move axes, sprint/crouch/aim/lean/fire |
| Character state | grounded, horizontal speed (m/s), a one-frame "landed" pulse |
| Raycasts and damage | Wall checks for auto-lean (core takes only a `lean_target`), bullet hits and damage go through the host |
| Audio | Core only emits events (footstep kind, later gunshots); the host plays sounds |
| Unit scale | cm vs m; the core is metres |

The host keeps owning movement, collision and the control rotation. The core reports `ctl_delta_*` (look plus recoil) so
the adapter can add the recoil part to the host's control rotation.

## Assets and copyright

The repo contains only our own measurements (`assets/camera_motion.json`, `fire_shake.json`: curves recorded from running the
game) and code. Game meshes, textures, animations and sounds belong to the Bodycam developers. They must come from the user's
own Bodycam install through our extraction pipeline (`tools/extract`) at install time. Never bundle, commit or redistribute
them in a mod package.

## Slice 1 (done): camera motion

`bodycam-core` ports idle pose, gait bob (walk/sprint/crouch/ADS loops), land event, lean (3 stage cascade), spring roll,
smooth control rotation lag, controller recoil (smoothing, damping, compensation), and fire shake. Tests compare it frame by frame
with the game's `player.rs` over a 6 second walk/sprint/land/fire/crouch/lean sequence.

Note: `bodycam-core` embeds `assets/camera_motion.json` and `fire_shake.json` with `include_str!`; if the data moves, update the paths.

## Slices 2-6 (done): everything pure

Rust module / C prefix / what it does (all in `bodycam_core.h`; each is tested, and `game/src/core_parity.rs` runs the game's own
code against the core on the same inputs, so they cannot drift apart silently):

| Module | C prefix | Slice |
|---|---|---|
| `weapon_data` | `bc_stats_*`, `bc_weapon_*` | Loads `assets/weapon_stats.json` (Default row folded in) and `assets/weapons/<Row>.json` from text the host passes. Nothing is embedded: the files come from the user's own install via `tools/extract`. |
| `sway` | `bc_sway_*` | ADS progress (AdsSpeed, sinusoidal ease), look/move spring sway, LowReadySway. Output: arms-root transform and `ads_alpha`. |
| `recoil` | `bc_recoil_*` | Weapon (gun kick) recoil animation: curves, noise, pushback, Z progress, recoil sway, fire sway. Output: arms-root transform. Camera/controller recoil is `bc_fire` from slice 1. |
| `gunfire` | `bc_gun_*` | Fire-rate cooldown, full/semi auto, dry fire, ammo, regrip roll on trigger release, elliptical spread cone per pellet. |
| `ballistics` | `bc_penetrate`, `bc_bone_multiplier`, `bc_surface_absorbance`, `bc_hit_damage` | GA_Fire_Base hit loop. Note: the game has no distance falloff; damage only changes through penetration and the bone hit. |
| `cues` | `bc_cues_*` | Footstep surface row (with Default fallback) plus foley layers, and weapon cue lookup, from the baked `audio.json` tables. Returns names only. |
| `lens` | `bc_lens_*`, `bc_vertical_fov_deg` | `LensParams` presets (game / ReShade; a test checks the ReShade preset equals the uniform defaults in `Bodycam.fx`), exposure-pump spring, H-FOV to V-FOV. |

Not in the core (needs the engine): mesh/bone driving, raycasts and the muzzle origin, animation montage playback and notifies,
reload/inventory rules, magnified-optic sway, `postfx.wesl` itself (use `mods/reshade/Bodycam.fx` or port the shader).

### Per-frame recipe (any engine)

```
every frame:
  step   = bc_step(core, input)                       -> camera pos/rot, footstep event
  if step.footstep: names = bc_cues_footstep(cues, kind, stride_start, surface_index) -> play each
  bc_gun_step(gun, dt, now, flags)                    -> fired / dry / regrip_delay
  if fired: bc_fire(core, ...bc_weapon_controller_recoil...)   // camera recoil
            bc_recoil_play(recoil, weapon, aiming)             // gun kick
            n = bc_gun_dirs(gun, muzzle_dir, aiming, dirs)     // host raycasts each dir
            for each dir: hits -> bc_penetrate(...) -> bc_hit_damage(base, mult, bone_name)
            bc_cues_weapon(cues, row, "Fire"/"FireSuppressed"/"FireDry", ...) -> play
  bc_sway_step(sway, weapon, look, velocity, aiming)  -> arms transform A, ads_alpha
  bc_recoil_tick(recoil, dt)                          -> arms transform B
  arms root = A then B (the game applies sway offset and recoil offset to the same root)
  lens: bc_lens_motion_step(m, dt, camera_angular_velocity) -> yaw_rate, pitch_rate, pump into your shader
```

Data: read the JSON files in the host (any language), pass the whole text. One `BcStats` can serve every weapon; make one
`BcSway`/`BcRecoil`/`BcGun` per equipped weapon (recreate the sway when the weapon changes).

### Engines

- **UE4SS C++**: `LoadLibrary("bodycam_core.dll")` + `GetProcAddress`, or link `bodycam_core.dll.lib`; include the header. Units: cm = m * 100,
  axes x right, y up, z back become UE (X fwd, Y right, Z up) as `(-z, x, y)`. Read weapon JSON with `std::ifstream`.
- **Unity** (BepInEx / MelonLoader): P/Invoke the same names (`[DllImport("bodycam_core")]`), `StringBuilder` for the `char*` outputs,
  `[StructLayout(LayoutKind.Sequential)]` structs matching the header. Unity is left handed with z forward: negate z, flip the sign of yaw
  and roll. Put the dll in the plugin folder (`Plugins/x86_64` for a normal build).
- **Godot 4**: a small GDExtension (Rust `libloading` or C++) wrapping the C functions, or call them directly from C# with `NativeLibrary`.
  Godot is right handed with -Z forward, x right, y up: the public convention is already that, only radians vs degrees and the
  character basis differ.
- **Anything else**: native plugin or injected DLL calling the C ABI; no callbacks, no allocation from the host side, handles are plain pointers
  you must destroy.

Build: `cargo build -p bodycam-core --release` gives `target/release/bodycam_core.dll` (`.so` on Linux). The header is hand-written
(no cbindgen dependency); `ffi::tests::header_declares_every_export` fails if an export is missing from it.

## Roadmap

1. Packaging: an installer script that runs the extraction against the user's Bodycam install, drops the core dll, the adapter, and the ReShade file.
2. Optional: magnified optic sway, spread bloom/other fire modes (burst) if the DT data gets them, a cbindgen header.

## UE4SS example adapter (UNTESTED)

`mods/ue4ss/BodycamCam/` contains a skeleton C++ mod (UE4SS Lua cannot call C ABI libraries directly), `src/dllmain.cpp`, which

1. loads `bodycam_core.dll` and resolves the `bc_*` functions,
2. each tick reads from the host: control rotation, pawn velocity, `CharacterMovement` grounded/crouched, mouse input axes,
3. calls `bc_step`, then writes the result to the camera via `PlayerCameraManager` `ViewTarget.POV` (location/rotation) in a camera update hook.

Documented as untested because: no Unreal game was available here, nothing was compiled against UE4SS, and the hook point
(`APlayerCameraManager::UpdateCamera` / `BlueprintUpdateCamera`) and property names differ by engine version and game. Treat it as a
design sketch to complete against a real target.
