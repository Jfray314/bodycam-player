"""Loose textures the game loads from assets-local (never from git): fx sprites/decals, light cookies, lens overlays, tablet UI art,
drone HUD art, attachment/gear icons, optic reticle masks, generated icons for parts without one.

  python -I tools/export_textures.py [fx|light|ui|drone|icons|reticles|genicons ...]     (default: all but genicons; run genicons after attachments)

Every texture is read straight from the game paks by ue-export (`tex` = full asset path, `texn` = bare asset name) and written
to assets-local. Steps skip files that already exist. Needs the same env as the other tools (BODYCAM_GAME, BODYCAM_EXTRACTED, BODYCAM_ASSETS).
"""
import glob, json, os, subprocess, sys, tempfile
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GAME = os.environ.get("BODYCAM_GAME", r"C:\Program Files (x86)\Steam\steamapps\common\Bodycam")
EXTRACTED = os.environ.get("BODYCAM_EXTRACTED", ROOT + "/extracted")
ASSETS = os.environ.get("BODYCAM_ASSETS", ROOT + "/assets-local")
BLENDER = os.environ.get("BLENDER", r"C:\Program Files\Blender Foundation\Blender 5.2\blender.exe")
DOTNET = os.environ.get("DOTNET", os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe"))
EXE = os.path.join(ROOT, "tools", "ue-export", "bin", "Release", "net10.0", "ue-export.dll")
C = "Bodycam/Content/"


def ue(out, cmd, names):
    names = [n for n in names if n]
    for i in range(0, len(names), 200):  # one ue-export start costs a few seconds: batch, but keep the command line short
        r = subprocess.run([DOTNET, EXE, GAME + "/Bodycam/Content/Paks", EXTRACTED + "/Bodycam.usmap", out, cmd] + names[i:i + 200],
                           capture_output=True, text=True, cwd=os.path.dirname(EXE))
        bad = [l for l in (r.stdout + r.stderr).splitlines() if "missing" in l or "failed" in l or "not a texture" in l]
        if r.returncode or bad:
            print("\n".join(bad[:10]) or (r.stdout + r.stderr)[-600:])


def tmp_export(paths):  # asset paths -> png in a scratch dir, returns {name: path}
    d = tempfile.mkdtemp(prefix="bc_tex_")
    ue(d, "tex", paths)
    return {os.path.basename(p)[:-4]: p for p in glob.glob(d + "/*.png")}


def todo_named(names, dest):
    os.makedirs(dest, exist_ok=True)
    need = [n for n in names if not os.path.exists(f"{dest}/{n}.png")]
    if need:
        ue(dest, "texn", need)
    print(f"  {dest}: {len(names) - len(need)} present, {len(need)} exported")


def fx():
    d = ASSETS + "/textures/fx"
    os.makedirs(d, exist_ok=True)
    B, DEC, WH = C + "BloodFX/Textures/", C + "BallisticsVFX/Decals/Textures/", C + "BallisticsVFX/Particles/FXMaterials/"
    # name -> (source asset, post-process)
    def rgb_to_alpha_g(im, size):  # white sprite whose alpha is the source's G channel (UE mask textures)
        g = im.convert("RGBA").getchannel("G").resize((size, size), Image.LANCZOS)
        return Image.merge("RGBA", (Image.new("L", g.size, 255),) * 3 + (g,))
    jobs = {
        "flare_star": (WH + "Flares/Textures/Flare_star", lambda im: im),
        "decal_blood": (DEC + "Splasher_1", lambda im: im.resize((1024, 1024), Image.LANCZOS)),
        "decal_concrete": (DEC + "Concrete_1", lambda im: im.resize((512, 512), Image.LANCZOS)),
        "decal_metal": (DEC + "Metal_1", lambda im: im.resize((512, 512), Image.LANCZOS)),
        "decal_wood": (DEC + "Wood_1", lambda im: im.resize((512, 512), Image.LANCZOS)),
        "blood_pool_2": (B + "T_BloodPoolMask2", lambda im: rgb_to_alpha_g(im, 512)),
        "blood_pool_3": (B + "T_BloodPoolMask3", lambda im: rgb_to_alpha_g(im, 512)),
        "blood_pool_4": (B + "T_BloodPoolMask4", lambda im: rgb_to_alpha_g(im, 512)),
        "blood_splat_3x3": (B + "T_Splatter2_Mask_Flipbook", lambda im: rgb_to_alpha_g(im, 512)),
        "lens_dirt": (C + "PostProcess/Textures/T_DirtMask_01", lambda im: im.convert("RGB").resize((1280, 720), Image.LANCZOS)),
        "screen_droplets": (C + "UltraDynamicSky/Textures/Weather/WaterDroplets_Small", lambda im: im.resize((512, 512), Image.LANCZOS)),
        "whisp_one_8x8": (WH + "SmokeDust/Textures/whisp_One_512_8x8", lambda im: im.resize((1024, 1024), Image.LANCZOS)),
        # frame (4,3) of the 8x8 whisp sheet = the smoke / dust sprite
        "smoke": (WH + "SmokeDust/Textures/whisp_One_512_8x8", lambda im: im.crop((4 * 256, 3 * 256, 5 * 256, 4 * 256))),
    }
    todo = {k: v for k, v in jobs.items() if not os.path.exists(f"{d}/{k}.png")}
    src = tmp_export(sorted({v[0] for v in todo.values()}))
    for k, (asset, post) in todo.items():
        name = os.path.basename(asset)
        if name in src:
            post(Image.open(src[name])).save(f"{d}/{k}.png")
        else:
            print("  missing source", asset)
    # LENS_VEIW_05 (lens-flare overlay texture living in a Blueprints folder)
    if not os.path.exists(ASSETS + "/textures/LENS_VEIW_05.png"):
        ue(ASSETS + "/textures", "tex", [C + "LensAndFilmPostProcess/Blueprints/LENS_VEIW_05"])
    print("  fx done")


def light():
    d = ASSETS + "/textures/light"
    os.makedirs(d, exist_ok=True)
    P = C + "LightProfilePack/Lighting/Profiles/"
    ue(d, "tex", [P + n for n in ("T_Flashlight_Mask_D", "T_Flashlight_Mask_D2", "cookie_2", "t_flashlightpattern", "telechargement__2_1") if not os.path.exists(f"{d}/{n}.png")])


UI_TEX = """BannerBodycam T_UI_ArrowLeft T_UI_ArrowRight T_UI_BannerBodybomb T_UI_BannerDeathmatch T_UI_BannerGunGame T_UI_BannerHardPoint
T_UI_BannerTeamDeathmatch T_UI_BannerVersus T_UI_BannerWingma T_UI_BannerZombie T_UI_Bug T_UI_Checkmark T_UI_CrossIcon T_UI_Disabled T_UI_Discord
T_UI_ELOCurrency T_UI_Enabled T_UI_ExitIcon T_UI_GM_Bodybomb T_UI_GM_Deathmatch T_UI_GM_Duel T_UI_GM_GunGame T_UI_GM_Hard_Point T_UI_GM_TeamDeathmatch
T_UI_GM_Training T_UI_GM_Versus T_UI_GM_Wingman T_UI_GM_Zombies T_UI_Leader T_UI_PlaylistIcon T_UI_ProfilePicture T_UI_Rarity0 T_UI_Rarity2
T_UI_RefreshIcon T_UI_ReissadCurrency T_UI_SettingsIcon T_UI_SocialIcon T_UI_Tag T_UI_Timer T_UI_Vote T_UI_Youtube
Weapons_Default_AK47 Weapons_Default_BK-102S Weapons_Default_Benelli Weapons_Default_CZ-P-09 Weapons_Default_Crossbow Weapons_Default_Deagle
Weapons_Default_Dragunov Weapons_Default_FAL Weapons_Default_Fostech Weapons_Default_Glock17 Weapons_Default_Kobra Weapons_Default_Kriss_Vector
Weapons_Default_M1911 Weapons_Default_M4A1 Weapons_Default_MP5 Weapons_Default_MP9 Weapons_Default_Mossberg Weapons_Default_Remington700
Weapons_Default_Revolver Weapons_Default_SG5-KF Weapons_Default_Scar Weapons_Default_UZI""".split()
# gear / grenade / drone icons shown by the tablet's SWAP lists (game/src/ui/screens.rs GEAR)
GEAR_ICONS = "BladeBaseReceiver DefaultOperator_02 FoldingBaseReceiver GrenadeFlash GrenadeFlash9-Bangers GrenadeG67 GrenadeG68 GrenadeN67 GrenadeRGN Icon_Kyoshin Icon_Trataxes Kamikaze KarambitBaseArcticEchoez KnifeBaseReceiver Sentinel".split()


def ui():
    todo_named(UI_TEX, ASSETS + "/ui/tex")


def drone():
    d = ASSETS + "/ui/drone"
    os.makedirs(d, exist_ok=True)
    ue(d, "tex", [C + "FPV_Drone/UI/" + n for n in ("Calque_65", "DroneFpvCrossAirBluredHeavy", "KMH_Blured", "T_Signal_1", "T_Signal_2", "T_Signal_3", "T_Signal_4")
                  if not os.path.exists(f"{d}/{n}.png")])


def icons():
    meta = os.path.join(ASSETS, "models", "_work_att", "attall_meta.json")  # tools/attall.py (run by build_attachments.py)
    names = set(GEAR_ICONS)
    if os.path.exists(meta):
        names |= {v["icon"] for v in json.load(open(meta))["items"].values() if v["icon"]}
    else:
        print("  no attall_meta.json yet: only the gear icons (run tools/build_attachments.py, then this step again)")
    todo_named(sorted(names), ASSETS + "/ui/tex/icons")


def genicons():  # attachments without a DT_WeaponSkins icon: render the part itself (the tracked assets/attachments.json names them gen_<glb>)
    d = ASSETS + "/ui/tex/icons"
    items = json.load(open(ROOT + "/assets/attachments.json"))["items"]
    glbs = sorted({v["icon"][4:] for v in items.values() if v.get("icon", "").startswith("gen_")})
    todo = [f"{ASSETS}/models/attachments/{g}.glb" for g in glbs if not os.path.exists(f"{d}/gen_{g}.png") and os.path.exists(f"{ASSETS}/models/attachments/{g}.glb")]
    if todo:
        r = subprocess.run([BLENDER, "--background", "--python", ROOT + "/tools/blender/icon_render.py", "--", d] + todo, capture_output=True, text=True)
        if r.returncode:
            print((r.stdout + r.stderr)[-600:])
    print(f"  generated icons: {len(glbs) - len(todo)} present, {len(todo)} rendered")


def reticles():  # optic reticle masks named by the tracked assets/attachments.json
    d = ASSETS + "/models/attachments/reticles"
    os.makedirs(d, exist_ok=True)
    items = json.load(open(ROOT + "/assets/attachments.json"))["items"]
    texs = {e["reticle"]["tex"]: e["reticle"]["tex_path"] for e in items.values() if "reticle" in e}
    ue(d, "tex", [C + p[len("/Game/"):].rsplit(".", 1)[0] for t, p in texs.items() if not os.path.exists(f"{d}/{t}.png")])
    for t in texs:
        f = f"{d}/{t}.png"
        if not os.path.exists(f):
            print("  reticle texture missing:", t)
        elif (px := Image.open(f)).mode != "RGBA" or min(px.getchannel("A").getextrema()) > 0 and px.convert("RGB").getextrema() != ((255, 255),) * 3:
            # raw export (RGB, or RGBA with an opaque alpha): black-background mask -> white + alpha = luminance (512 px), what attach.rs expects
            im = px.convert("RGB").resize((512, 512), Image.LANCZOS)
            a = im.convert("L").point(lambda v: min(255, v * 2))
            Image.merge("RGBA", (Image.new("L", im.size, 255),) * 3 + (a,)).save(f)


if __name__ == "__main__":
    steps = sys.argv[1:] or ["fx", "light", "ui", "drone", "icons", "reticles"]
    for s in steps:
        print("export_textures:", s, flush=True)
        globals()[s]()
