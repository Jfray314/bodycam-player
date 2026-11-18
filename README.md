# Bodycam player (Rust / Bevy)

A 1:1 rewrite of the first-person player of the Unreal Engine game **Bodycam**: movement, camera / lens, guns, viewmodel, attachments, grenades,
drones, RC cars, tablet menu, audio, effects, and the real lobby map. Everything is ported from the real game's cooked files rather than
approximated (see [REWRITE.md](REWRITE.md) for the pipeline, the source-to-Rust map and the open items).

## No game content in this repository

This repo contains **code and small baked number tables only** (`assets/*.json`: weapon stats, recoil, sockets, attachment data...).
Meshes, textures, audio, fonts, video and maps are **never** committed: you must own Bodycam, and a setup script builds them from
*your own install* (read-only; nothing is ever written to the game folder). They land in two git-ignored folders:
`extracted/` (raw dumps) and `assets-local/` (what the game loads).

## Requirements (Windows)

| What | Why | Get it |
|---|---|---|
| Bodycam (Steam) | the source of every asset | your Steam library |
| Rust (stable) | the game and the Rust tools | https://rustup.rs |
| Python 3.9+ with `pillow numpy` | setup and the bake scripts | `pip install pillow numpy` |
| .NET 10 SDK | `ue-export` (CUE4Parse) reads the paks | https://dotnet.microsoft.com/download/dotnet/10.0 |
| Blender 4.2+ (tested 5.2) + `io_scene_psk_psa` extension | builds the `.glb` models | `blender --background --online-mode --command extension install --enable io_scene_psk_psa` |
| Visual Studio Build Tools, "Desktop development with C++" | builds `CUE4Parse-Natives.dll` (animation decompression), once | https://visualstudio.microsoft.com/downloads/ |
| git | NihAV download (VHS screen), natives build | https://git-scm.com |
| `extracted/Bodycam.usmap` | Unreal type mappings, made from the *running* game | see below |

`setup` checks all of this first and tells you exactly what is missing.

### The one manual step: `Bodycam.usmap`

The cooked assets are "unversioned", so they can only be read with the game's own type mappings. Those come from UE4SS, once:

1. `powershell -ExecutionPolicy Bypass -File tools\ue4ss_probe\install.ps1 -Game "<Bodycam install dir>"` (installs UE4SS into your game folder;
   undo with `-Uninstall`), or install UE4SS "experimental" (UE 5.5 support) yourself.
2. Start Bodycam, get into the lobby, and in the UE4SS window use *Dumpers > Generate .usmap file* (keybind Ctrl+Numpad6).
   It writes a `.usmap` next to `UE4SS.dll` in `Bodycam\Binaries\Win64`.
3. Copy it to `extracted\Bodycam.usmap` in this checkout.

## Setup

```
setup.bat
:: or
python tools/setup.py --game "D:\SteamLibrary\steamapps\common\Bodycam"
```

The game folder is found automatically through Steam's library list, or `BODYCAM_GAME`, or `--game`. The script runs the whole pipeline
(build tools, extract, dump, fonts, audio, models, items, RC cars, attachments, textures, map, VHS atlas), prints progress, and each
step skips itself when its outputs exist, so an interrupted run can simply be started again. `--list` shows the steps, `--only a,b` / `--skip a,b`
select, `--force` redoes. It takes a while (hundreds of Blender and CUE4Parse runs) and needs roughly 10 GB of disk.

## Run

```
cargo run --release -p game
```

(`play.bat` does the same and opens straight into the lobby.) To keep the generated assets elsewhere set `BODYCAM_ASSETS=<dir>` for both
setup (`--assets`) and the game.

## Controls

WASD move, Shift sprint, Left Ctrl crouch (hold), Space jump, Q / E lean, LMB fire, RMB aim, R tap reload (hold 0.25 s: mag check),
J inspect, 1-5 / mouse wheel swap (primary, secondary, grenade, melee, perk), X quick knife attack, G drop item, F pick up,
C flashlight, Z laser, MMB switch optic, Esc / Tab raise the tablet menu (loadouts, attachments, world picker),
F2 toggle the camera post-process. In a drone / RC car: F leaves, C drone light.

## Layout

- `game/` the Bevy game; `bodycam-core/` reusable core lib with a C header
- `assets/` baked numeric tables compiled into the game (tracked)
- `tools/` the pipeline: `setup.py` driver, `ue-export` (C#, CUE4Parse), `uasset` / `extract` / `binka` / `audio-bake` (Rust), Blender scripts, bake scripts
- `mods/` optional UE4SS / ReShade mods for the original game; `docs/MODDING.md`

Bodycam and its content belong to their owners; this project is unofficial and ships none of it.
