# BodycamCam (UE4SS C++ mod skeleton, UNTESTED)

Loads `bodycam_core.dll` (build: `cargo build -p bodycam-core --release`) and drives the host camera each tick.
Install idea: put the compiled mod dll as `Mods/BodycamCam/dlls/main.dll`, `bodycam_core.dll` in `Mods/BodycamCam/`,
and an empty `enabled.txt`. Nothing here was compiled or run: no UE4SS SDK or Unreal game was available. The hook points
and property names in `src/dllmain.cpp` are placeholders. See `docs/MODDING.md`.
