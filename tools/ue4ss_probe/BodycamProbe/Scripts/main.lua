-- BodycamProbe: reads live values the asset files don't contain (C++ defaults, runtime camera motion).
-- F7 = dump properties of player/camera/movement objects
-- F8 = start/stop recording the final camera view every frame-ish (CSV)
-- F9 = drop a numbered marker into the recording (to split segments)
-- Output: C:/code/bodycam-player/extracted/probe/
local UEHelpers = require("UEHelpers")
local OUT = "C:/code/bodycam-player/extracted/probe/"

local function fmt(v, depth)
    depth = depth or 0
    local t = type(v)
    if t == "number" or t == "boolean" or t == "string" then return tostring(v) end
    if t ~= "userdata" or depth > 1 then return "<" .. t .. ">" end
    for _, f in ipairs({ "ToString", "GetFullName" }) do
        local ok, r = pcall(function() return v[f](v) end)
        if ok and type(r) == "string" then return r end
    end
    local ok, r = pcall(function()
        if v.X ~= nil then return string.format("(%s,%s,%s)", tostring(v.X), tostring(v.Y), tostring(v.Z)) end
        if v.Pitch ~= nil then return string.format("(P=%s,Y=%s,R=%s)", tostring(v.Pitch), tostring(v.Yaw), tostring(v.Roll)) end
    end)
    if ok and r then return r end
    return "<userdata>"
end

local function dump_object(f, obj)
    if obj == nil or not obj:IsValid() then return end
    f:write("== " .. obj:GetFullName() .. "\n")
    local cls = obj:GetClass()
    while cls ~= nil and cls:IsValid() do
        local cname = cls:GetFName():ToString()
        f:write("  -- class " .. cname .. "\n")
        pcall(function()
            cls:ForEachProperty(function(prop)
                local name = prop:GetFName():ToString()
                local ptype = prop:GetClass():GetFName():ToString()
                local ok, val = pcall(function() return obj[name] end)
                f:write(string.format("  %s : %s = %s\n", name, ptype, ok and fmt(val) or "<err>"))
            end)
        end)
        if cname == "Object" or cname == "Actor" or cname == "ActorComponent" then break end
        cls = cls:GetSuperStruct()
    end
end

local CLASSES = {
    "Bodycam_Player_C", "CharacterMovementComponent", "BP_BodycamPlayerCameraManager_C",
    "AC_BodycamCamera_C", "AC_BodycamAnimationComponent_C", "CameraComponent",
    "ALS_AnimBP_C", "Viewmodel_C", "ABP_Procedural_GoPro_C", "ABP_Procedural_Sway_C",
    "ABP_Procedural_Viewmodel_C", "ABP_Procedural_Additives_C", "ABP_Procedural_AimOffset_C",
    "Bodycam_CameraShake_C", "Sprint_CameraShake_C", "WeaponAttributeSet", "BP_EquippableItem_C",
}

local function dump_all()
    local f = io.open(OUT .. "dump_" .. os.time() .. ".txt", "w")
    if not f then print("[BodycamProbe] cannot open output dir " .. OUT) return end
    for _, c in ipairs(CLASSES) do
        local all = FindAllOf(c)
        f:write("\n######## " .. c .. " (" .. (all and #all or 0) .. " instances)\n")
        if all then
            for i, o in ipairs(all) do
                if i > 4 then break end
                local ok, err = pcall(dump_object, f, o)
                if not ok then f:write("  <dump error " .. tostring(err) .. ">\n") end
            end
        end
    end
    f:close()
    print("[BodycamProbe] dump written")
end

local rec, marker, start = nil, 0, 0

local function sample()
    local pc = UEHelpers.GetPlayerController()
    if not pc or not pc:IsValid() then return end
    local cm, pawn = pc.PlayerCameraManager, pc.Pawn
    if not cm or not cm:IsValid() or not pawn or not pawn:IsValid() then return end
    local cl, cr, fov = cm:GetCameraLocation(), cm:GetCameraRotation(), cm:GetFOVAngle()
    local pl, pv, ctl = pawn:K2_GetActorLocation(), pawn:GetVelocity(), pc:GetControlRotation()
    local mv = pawn.CharacterMovement
    local crouched = pawn.bIsCrouched and 1 or 0
    local falling = (mv and mv:IsValid() and mv:IsFalling()) and 1 or 0
    rec:write(string.format("%.4f,%d,%.3f,%.3f,%.3f,%.4f,%.4f,%.4f,%.3f,%.3f,%.3f,%.3f,%.3f,%.3f,%.4f,%.4f,%.4f,%.3f,%d,%d\n",
        os.clock() - start, marker, cl.X, cl.Y, cl.Z, cr.Pitch, cr.Yaw, cr.Roll, fov,
        pl.X, pl.Y, pl.Z, pv.X, pv.Y, pv.Z, ctl.Pitch, ctl.Yaw, ctl.Roll, crouched, falling))
end

local function toggle_record()
    if rec then
        local f = rec
        rec = nil
        f:close()
        print("[BodycamProbe] recording stopped")
        return
    end
    rec = io.open(OUT .. "camera_" .. os.time() .. ".csv", "w")
    if not rec then print("[BodycamProbe] cannot open output dir " .. OUT) return end
    rec:write("t,marker,cam_x,cam_y,cam_z,cam_pitch,cam_yaw,cam_roll,fov,pawn_x,pawn_y,pawn_z,vel_x,vel_y,vel_z,ctl_pitch,ctl_yaw,ctl_roll,crouched,falling\n")
    marker, start = 0, os.clock()
    print("[BodycamProbe] recording started")
    LoopAsync(8, function()
        if not rec then return true end
        ExecuteInGameThread(function() if rec then pcall(sample) end end)
        return false
    end)
end

RegisterKeyBind(Key.F7, function() ExecuteInGameThread(function() pcall(dump_all) end) end)
RegisterKeyBind(Key.F8, function() ExecuteInGameThread(toggle_record) end)
RegisterKeyBind(Key.F9, function() marker = marker + 1 print("[BodycamProbe] marker " .. marker) end)
print("[BodycamProbe] loaded: F7 dump, F8 record, F9 marker")
