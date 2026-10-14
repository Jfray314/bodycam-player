// UNTESTED SKELETON. UE4SS C++ mod that drives the host camera from bodycam-core.
// Build as a UE4SS C++ mod (see RE-UE4SS docs); link nothing else, bodycam_core.dll is loaded at runtime.
// Property/function names below are placeholders: verify against the target game's UE version (UE4SS object dumper).
#include <Mod/CppUserModBase.hpp>
#include <Unreal/UObjectGlobals.hpp>
#include <Unreal/UObject.hpp>
#include <Unreal/UFunction.hpp>
#include <windows.h>
#include <cstdint>

extern "C" {
struct BcInput { float dt, look_yaw_deg, look_pitch_deg, move_x, speed, lean_target; uint32_t flags; };
struct BcOutput {
    float pos[3], rot[3], base_pos[3], base_rot[3];
    float ctl_yaw_deg, ctl_pitch_deg, ctl_delta_yaw_deg, ctl_delta_pitch_deg, lean;
    int32_t footstep, stride_start;
};
}

using namespace RC;
using namespace RC::Unreal;

class BodycamCam : public RC::CppUserModBase
{
    HMODULE dll{};
    void* core{};
    void* (*bc_create)(uint64_t){};
    void (*bc_destroy)(void*){};
    void (*bc_step)(void*, const BcInput*, BcOutput*){};

public:
    BodycamCam()
    {
        ModName = STR("BodycamCam");
        dll = LoadLibraryW(L"Mods\\BodycamCam\\bodycam_core.dll");
        if (!dll) return;
        bc_create = (decltype(bc_create))GetProcAddress(dll, "bc_create");
        bc_destroy = (decltype(bc_destroy))GetProcAddress(dll, "bc_destroy");
        bc_step = (decltype(bc_step))GetProcAddress(dll, "bc_step");
        if (bc_create && bc_step) core = bc_create(GetTickCount64());
    }
    ~BodycamCam() override
    {
        if (core && bc_destroy) bc_destroy(core);
        if (dll) FreeLibrary(dll);
    }

    auto on_update() -> void override
    {
        if (!core) return;
        // TODO per tick, using UObjectGlobals::FindFirstOf / property reads (names are game specific):
        //  1. PlayerController: control rotation, mouse axis deltas (degrees), pawn velocity (cm/s -> m/s), IsFalling, IsCrouched.
        //  2. Fill BcInput (dt from the engine delta; flags: GROUNDED, CROUCHED, SPRINTING, AIMING, ...).
        //  3. bc_step(core, &in, &out).
        //  4. Write the camera: PlayerCameraManager.ViewTarget.POV.Location = pawn feet + (out.pos, x100 for cm, axes: x right -> Y,
        //     y up -> Z, z back -> -X in the pawn's frame); POV.Rotation = control rotation + (out.rot). Do it in a post
        //     UpdateCamera hook (RegisterHook on /Script/Engine.PlayerCameraManager:UpdateCamera) so the host cannot overwrite it.
        //  5. Add out.ctl_delta_* recoil part to the control rotation if the host should aim with the recoil too.
        //  6. Hide the host's first-person mesh (SetHiddenInGame on its viewmodel component).
    }
};

#define BODYCAM_CAM_API __declspec(dllexport)
extern "C" {
BODYCAM_CAM_API RC::CppUserModBase* start_mod() { return new BodycamCam(); }
BODYCAM_CAM_API void uninstall_mod(RC::CppUserModBase* mod) { delete mod; }
}
