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

## Roadmap

1. Viewmodel pose: port `sway.rs` and the viewmodel layer of `viewmodel.rs` (weapon sway, stress, ADS blend, regrip additives) so the core
   outputs arm/gun root transforms per frame. Host provides the arms mesh bone driving (the animation clips stay in the
   extracted assets; the core only outputs poses/blend weights).
2. ADS: aim alpha, FOV handling (Bodycam keeps FOV constant at 120 horizontal; distortion is post), sensitivity scaling.
3. Gunfire and ballistics: `weapon.rs` fire rates, spread, ballistic model; core outputs shots (origin, direction, weapon row)
   and the host does the raycasts and applies damage. Muzzle flash, ejection sockets as events.
4. Audio events: footsteps (already emitted as `footstep`), gunshot, reload, ADS. Event IDs plus the sound names; the host
   plays them from the extracted audio.
5. Packaging: an installer script that runs the extraction against the user's Bodycam install, drops the core dll, the adapter, and the ReShade file.

## UE4SS example adapter (UNTESTED)

`mods/ue4ss/BodycamCam/` contains a skeleton C++ mod (UE4SS Lua cannot call C ABI libraries directly), `src/dllmain.cpp`, which

1. loads `bodycam_core.dll` and resolves the `bc_*` functions,
2. each tick reads from the host: control rotation, pawn velocity, `CharacterMovement` grounded/crouched, mouse input axes,
3. calls `bc_step`, then writes the result to the camera via `PlayerCameraManager` `ViewTarget.POV` (location/rotation) in a camera update hook.

Documented as untested because: no Unreal game was available here, nothing was compiled against UE4SS, and the hook point
(`APlayerCameraManager::UpdateCamera` / `BlueprintUpdateCamera`) and property names differ by engine version and game. Treat it as a
design sketch to complete against a real target.
