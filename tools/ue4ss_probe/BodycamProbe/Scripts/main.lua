-- BodycamProbe: reads live values the asset files don't contain (C++ defaults, runtime camera/viewmodel motion).
-- F7 = dump properties of player/camera/movement objects
-- F8 = start/stop recording camera + every numeric property of the viewmodel/recoil/status objects, every frame (CSV)
-- F9 = drop a numbered marker into the recording (to split segments)
-- Output: F:/code/bodycam-player/extracted/probe/
local UEHelpers = require("UEHelpers")
local OUT = "F:/code/bodycam-player/extracted/probe/"

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
    "RecoilAnimationComponent", "WW_SurvivorStatus_C",
}

local dumping = false
local function dump_all()
    -- One at a time (a second F7 while the first runs crashed), and only the local player's objects: dumping every
    -- bot's objects in a bot match crashed too.
    if dumping then return end
    local lp = UEHelpers.GetPlayerController()
    local me = lp and lp:IsValid() and lp.Pawn
    if not (me and me:IsValid()) then print("[BodycamProbe] player not found, spawn in first") return end
    local mine = me:GetFName():ToString()
    dumping = true
    local f = io.open(OUT .. "dump_" .. os.time() .. ".txt", "w")
    if not f then dumping = false print("[BodycamProbe] cannot open output dir " .. OUT) return end
    for _, c in ipairs(CLASSES) do
        local all = FindAllOf(c)
        f:write("\n######## " .. c .. " (" .. (all and #all or 0) .. " instances)\n")
        if all then
            local n = 0
            for _, o in ipairs(all) do
                local ok_name, full = pcall(function() return o:GetFullName() end)
                local own = ok_name and full and (full:find(mine, 1, true) or c == "WeaponAttributeSet" or c:find("CameraManager", 1, true))
                if own then n = n + 1 end
                if n > 4 then break end
                local ok, err = true, nil
                if own then ok, err = pcall(dump_object, f, o) end
                if not ok then f:write("  <dump error " .. tostring(err) .. ">\n") end
            end
        end
    end
    f:close()
    dumping = false
    print("[BodycamProbe] dump written")
end

-- Recording -----------------------------------------------------------------------------------------------
-- Property reads only: calling UFunctions (GetCameraLocation, GetSocketTransform...) from the tick crashed the game.
-- Objects and the column list are resolved once when recording starts, not every sample.

-- Objects whose numeric properties are recorded (class names; the instance owned by the local pawn is used).
-- Anim instances hold the BP-side state (ADS alpha, stress alphas, sway inputs) and the native AnimNode_SpringSway
-- structs; RecoilAnimationComponent is PRAS; WW_SurvivorStatus has stress/stamina/health.
local TRACKED = {
    "Viewmodel_C", "ABP_Procedural_Sway_C", "ABP_Procedural_Viewmodel_C", "ABP_Procedural_Additives_C",
    "ABP_Procedural_AimOffset_C", "ABP_Procedural_GoPro_C", "AC_BodycamAnimationComponent_C", "AC_BodycamCamera_C",
    "RecoilAnimationComponent", "WW_SurvivorStatus_C",
}
local NUMERIC = { FloatProperty = true, DoubleProperty = true, IntProperty = true, BoolProperty = true, ByteProperty = true, EnumProperty = true }
local LEAF_STRUCTS = { -- struct name -> fields recorded as numbers
    Vector = { "X", "Y", "Z" }, Vector2D = { "X", "Y" }, Rotator = { "Pitch", "Yaw", "Roll" },
    Quat = { "X", "Y", "Z", "W" }, Vector4 = { "X", "Y", "Z", "W" },
}
local MAX_DEPTH = 3       -- struct nesting (AnimNode_SpringSway -> its vectors)
local MAX_COLUMNS = 4000

local rec, marker, start, hooked = nil, 0, 0, false
local pc, cm, pawn, root, mv
local columns = {}        -- { obj, path = {names...}, leaf = field or nil, bool = true/false }

local function owned_by_pawn(o)
    local ok, name = pcall(function() return o:GetFullName() end)
    return ok and name and name:find(pawn:GetFName():ToString(), 1, true) ~= nil
end

local function struct_name(prop)
    local ok, s = pcall(function() return prop:GetStruct():GetFName():ToString() end)
    return ok and s or nil
end

-- Walks a struct/class's properties and appends a column per numeric leaf.
local function collect(obj, label, prop_owner, path, depth)
    pcall(function()
        prop_owner:ForEachProperty(function(prop)
            if #columns >= MAX_COLUMNS then return end
            local name = prop:GetFName():ToString()
            local ptype = prop:GetClass():GetFName():ToString()
            local p = {}
            for i, v in ipairs(path) do p[i] = v end
            p[#p + 1] = name
            if NUMERIC[ptype] then
                columns[#columns + 1] = { obj = obj, path = p, name = label .. "." .. table.concat(p, "."), bool = ptype == "BoolProperty" }
            elseif ptype == "StructProperty" then
                local s = struct_name(prop)
                if s and LEAF_STRUCTS[s] then
                    for _, f in ipairs(LEAF_STRUCTS[s]) do
                        columns[#columns + 1] = { obj = obj, path = p, leaf = f, name = label .. "." .. table.concat(p, ".") .. "." .. f }
                    end
                elseif s and depth < MAX_DEPTH then
                    collect(obj, label, prop:GetStruct(), p, depth + 1)
                end
            end
        end)
    end)
end

local function collect_object(obj, label)
    local cls = obj:GetClass()
    while cls ~= nil and cls:IsValid() do
        local cname = cls:GetFName():ToString()
        if cname == "Object" or cname == "Actor" or cname == "ActorComponent" or cname == "AnimInstance" or cname == "SceneComponent" then break end
        collect(obj, label, cls, {}, 0)
        cls = cls:GetSuperStruct()
    end
end

local function resolve()
    pc = UEHelpers.GetPlayerController()
    if not pc or not pc:IsValid() then return false end
    cm, pawn = pc.PlayerCameraManager, pc.Pawn
    if not cm or not cm:IsValid() or not pawn or not pawn:IsValid() then return false end
    root, mv = pawn.RootComponent, pawn.CharacterMovement
    if not (root ~= nil and root:IsValid() and mv ~= nil and mv:IsValid()) then return false end
    columns = {}
    for _, c in ipairs(TRACKED) do
        local all = FindAllOf(c)
        local n = 0
        for _, o in ipairs(all or {}) do
            if o:IsValid() and owned_by_pawn(o) then
                n = n + 1
                collect_object(o, c .. (n > 1 and tostring(n) or ""))
            end
        end
        print(string.format("[BodycamProbe] %s: %d instance(s)", c, n))
    end
    print("[BodycamProbe] columns: " .. #columns)
    return true
end

local function read(col)
    local v = col.obj
    for _, k in ipairs(col.path) do v = v[k] end
    if col.leaf then v = v[col.leaf] end
    if col.bool then return v and 1 or 0 end
    return v
end

-- Shift+F8: toggle recording world transforms of viewmodel sockets via SkeletalMeshComponent.GetSocketTransform (one UFunction
-- call per socket per frame). On by default (no F10 on the user's keyboard); an earlier probe crashed on UFunction calls.
local BONES = { "ik_hand_gun", "hand_r", "hand_l", "FP_Camera", "head" }
local bones_on = true
local function bone_cols(out)
    local mesh = bones_on and pawn.Mesh
    for _, b in ipairs(BONES) do
        local ok, t = false, nil
        if mesh and mesh:IsValid() then ok, t = pcall(function() return mesh:GetSocketTransform(FName(b), 0) end) end
        local vals = { "", "", "", "", "", "", "" }
        if ok and t then
            pcall(function()
                local l, q = t.Translation, t.Rotation
                vals = { string.format("%.3f", l.X), string.format("%.3f", l.Y), string.format("%.3f", l.Z),
                    string.format("%.6f", q.X), string.format("%.6f", q.Y), string.format("%.6f", q.Z), string.format("%.6f", q.W) }
            end)
        end
        for _, v in ipairs(vals) do out[#out + 1] = v end
    end
end

local function sample()
    if not (pc:IsValid() and cm:IsValid() and pawn:IsValid() and root:IsValid() and mv:IsValid()) then return end
    local pov = cm.CameraCachePrivate.POV
    local cl, cr, fov = pov.Location, pov.Rotation, pov.FOV
    local pl, pv, ctl = root.RelativeLocation, mv.Velocity, pc.ControlRotation
    local crouched = pawn.bIsCrouched and 1 or 0
    local falling = (mv.MovementMode == 3) and 1 or 0
    local row = { string.format("%.4f,%d,%.3f,%.3f,%.3f,%.4f,%.4f,%.4f,%.3f,%.3f,%.3f,%.3f,%.3f,%.3f,%.4f,%.4f,%.4f,%.3f,%d,%d",
        os.clock() - start, marker, cl.X, cl.Y, cl.Z, cr.Pitch, cr.Yaw, cr.Roll, fov,
        pl.X, pl.Y, pl.Z, pv.X, pv.Y, pv.Z, ctl.Pitch, ctl.Yaw, ctl.Roll, crouched, falling) }
    for i, col in ipairs(columns) do
        local ok, v = pcall(read, col)
        row[i + 1] = (ok and type(v) == "number") and string.format("%.5g", v) or ""
    end
    bone_cols(row)
    rec:write(table.concat(row, ","), "\n")
end

local function open_file(tag)
    rec = io.open(OUT .. "rec_" .. os.time() .. tag .. ".csv", "w")
    if not rec then print("[BodycamProbe] cannot open output dir " .. OUT) return false end
    local head = { "t,marker,cam_x,cam_y,cam_z,cam_pitch,cam_yaw,cam_roll,fov,pawn_x,pawn_y,pawn_z,vel_x,vel_y,vel_z,ctl_pitch,ctl_yaw,ctl_roll,crouched,falling" }
    for i, col in ipairs(columns) do head[i + 1] = (col.name:gsub(",", "_")) end
    for _, b in ipairs(BONES) do
        for _, c in ipairs({ "x", "y", "z", "qx", "qy", "qz", "qw" }) do head[#head + 1] = "socket." .. b .. "." .. c end
    end
    rec:write(table.concat(head, ","), "\n")
    return true
end

-- Called from every Bodycam_Player tick (bots too). After a death the controller possesses a new pawn: the old
-- objects go invalid, so re-resolve and continue in a new file (rec_<time>_respawn.csv), same clock and markers.
local function on_tick(self)
    if not rec then return end
    local me = self:get()
    local ok, same = pcall(function() return pawn:IsValid() and me:GetAddress() == pawn:GetAddress() end)
    if ok and same then pcall(sample) return end
    local ok2, mine = pcall(function()
        return pc:IsValid() and pc.Pawn and pc.Pawn:IsValid() and pc.Pawn:GetAddress() == me:GetAddress()
    end)
    if not (ok2 and mine) then return end
    rec:close()
    rec = nil
    if resolve() and open_file("_respawn") then print("[BodycamProbe] respawn: recording continues in a new file") end
end

local function toggle_record()
    if rec then
        local f = rec
        rec = nil
        f:close()
        print("[BodycamProbe] recording stopped")
        return
    end
    if not resolve() then print("[BodycamProbe] player not found, spawn in first") return end
    if not open_file("") then return end
    marker, start = 0, os.clock()
    print("[BodycamProbe] recording started")
    -- Sample from the player's own BP tick (game thread, once per frame). A LoopAsync +
    -- ExecuteInGameThread per sample crashed after ~35s inside UE4SS's callback cleanup.
    if not hooked then
        hooked = true
        RegisterHook("/BodycamAnimationFramework/Blueprints/Bodycam_Player.Bodycam_Player_C:ReceiveTick", function(self) pcall(on_tick, self) end)
    end
end

RegisterKeyBind(Key.F7, function() ExecuteInGameThread(function() pcall(dump_all) end) end)
RegisterKeyBind(Key.F8, function() ExecuteInGameThread(toggle_record) end)
RegisterKeyBind(Key.F8, { ModifierKey.SHIFT }, function() bones_on = not bones_on print("[BodycamProbe] socket transforms " .. (bones_on and "on" or "off")) end)
RegisterKeyBind(Key.F9, function() marker = marker + 1 print("[BodycamProbe] marker " .. marker) end)
print("[BodycamProbe] loaded: F7 dump, F8 record, F9 marker, Shift+F8 socket transforms on/off (default on)")
