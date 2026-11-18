"""One-command setup: builds everything the game loads from `extracted/` and `assets-local/` out of YOUR OWN Bodycam install.

    python tools/setup.py [--game "<Bodycam install dir>"] [--only a,b] [--skip a,b] [--force] [--list] [--check]

No game content is in this repository. This reads the game's .pak files read-only and writes only into this checkout
(`extracted/` raw dumps, `assets-local/` meshes / textures / audio / fonts / maps; both git-ignored). Every step skips itself when its
outputs already exist (resume after an interruption), `--force` redoes the selected steps. Set BODYCAM_EXTRACTED / BODYCAM_ASSETS
(or --extracted / --assets) to write somewhere else; run the game with BODYCAM_ASSETS=<that dir>.
"""
import argparse, glob, importlib.util, json, os, re, shutil, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOOLS = ROOT + "/tools"
DEFAULT_GAME = r"C:\Program Files (x86)\Steam\steamapps\common\Bodycam"

ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument("--game", help="Bodycam install dir (the folder that contains Bodycam\\Content\\Paks); default: $BODYCAM_GAME, else searched in Steam libraries")
ap.add_argument("--extracted", help="where to write extracted/ (default: <repo>/extracted, or $BODYCAM_EXTRACTED)")
ap.add_argument("--assets", help="where to write assets-local/ (default: <repo>/assets-local, or $BODYCAM_ASSETS)")
ap.add_argument("--only", default="", help="comma list of steps to run (see --list)")
ap.add_argument("--skip", default="", help="comma list of steps to skip")
ap.add_argument("--force", action="store_true", help="redo steps even when their outputs exist")
ap.add_argument("--list", action="store_true", help="list the steps and exit")
ap.add_argument("--check", action="store_true", help="only check prerequisites and exit")
args = ap.parse_args()

EXTRACTED = os.path.abspath(args.extracted or os.environ.get("BODYCAM_EXTRACTED") or ROOT + "/extracted").replace("\\", "/")
ASSETS = os.path.abspath(args.assets or os.environ.get("BODYCAM_ASSETS") or ROOT + "/assets-local").replace("\\", "/")
USMAP = EXTRACTED + "/Bodycam.usmap"


# ---------------------------------------------------------------- prerequisites
def steam_libraries():
    roots = []
    try:
        import winreg
        for hive, key in ((winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam"), (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam")):
            try:
                with winreg.OpenKey(hive, key) as k:
                    roots.append(winreg.QueryValueEx(k, "SteamPath" if hive == winreg.HKEY_CURRENT_USER else "InstallPath")[0])
            except OSError:
                pass
    except ImportError:
        pass
    roots += [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam"]
    libs = []
    for r in roots:
        libs.append(r)
        vdf = os.path.join(r, "steamapps", "libraryfolders.vdf")
        if os.path.exists(vdf):
            libs += [p.replace("\\\\", "\\") for p in re.findall(r'"path"\s+"([^"]+)"', open(vdf, encoding="utf8", errors="replace").read())]
    return libs


def find_game():
    cands = [args.game, os.environ.get("BODYCAM_GAME")] + [os.path.join(l, "steamapps", "common", "Bodycam") for l in steam_libraries()] + [DEFAULT_GAME]
    for c in cands:
        if c and os.path.isdir(os.path.join(c, "Bodycam", "Content", "Paks")):
            return os.path.abspath(c).replace("\\", "/")
    return None


def first_sdk10(cands):
    for c in cands:
        if not c:
            continue
        try:
            r = subprocess.run([c, "--list-sdks"], capture_output=True, text=True)
            if r.returncode == 0 and re.search(r"^10\.", r.stdout, re.M):
                return c
        except OSError:
            pass


def find_blender():
    cands = [os.environ.get("BLENDER"), shutil.which("blender")]
    for pf in (os.environ.get("ProgramFiles", r"C:\Program Files"), os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")):
        cands += sorted(glob.glob(os.path.join(pf, "Blender Foundation", "Blender *", "blender.exe")), reverse=True)
    cands += sorted(glob.glob(os.path.expanduser(r"~\AppData\Local\Programs\Blender Foundation\Blender *\blender.exe")), reverse=True)
    return next((c for c in cands if c and os.path.exists(c)), None)


def blender_has_psk(b):
    r = subprocess.run([b, "--background", "--python-expr", "import importlib.util as u; print('PSKPSA', u.find_spec('bl_ext.blender_org.io_scene_psk_psa') is not None)"],
                       capture_output=True, text=True)
    return "PSKPSA True" in r.stdout


def prereqs():
    """-> (ok, env for child processes). Prints every problem with how to fix it."""
    print("== prerequisites")
    bad = []

    def line(ok, what, fix=""):
        print(f"  [{'ok' if ok else 'MISSING'}] {what}")
        if not ok:
            bad.append(what)
            print("          " + fix.replace("\n", "\n          "))

    env = {"BODYCAM_EXTRACTED": EXTRACTED, "BODYCAM_ASSETS": ASSETS}
    game = find_game()
    line(game, f"Bodycam install: {game or 'not found'}",
         "Pass --game \"<folder containing Bodycam\\Content\\Paks>\" or set BODYCAM_GAME (this repo contains no game files; you must own Bodycam).")
    if game:
        env["BODYCAM_GAME"] = game
    line(sys.version_info >= (3, 9), f"Python {sys.version.split()[0]} (3.9+)", "Install Python 3.9+ from https://www.python.org/downloads/")
    mods = [m for m in ("PIL", "numpy") if importlib.util.find_spec(m) is None]
    line(not mods, "Python packages Pillow + numpy", "pip install pillow numpy")
    cargo = shutil.which("cargo")
    line(cargo, "Rust (cargo)", "Install Rust from https://rustup.rs/ (stable toolchain)")
    line(shutil.which("git"), "git", "Install Git for Windows from https://git-scm.com/")
    dotnet = first_sdk10([os.environ.get("DOTNET"), os.path.expanduser(r"~\.dotnet-sdk\dotnet.exe"), shutil.which("dotnet")])
    line(dotnet, f".NET 10 SDK ({dotnet or 'not found'})",
         "Install the .NET 10 SDK: https://dotnet.microsoft.com/download/dotnet/10.0\n(or: powershell -c \"& ([scriptblock]::Create((irm https://dot.net/v1/dotnet-install.ps1))) -Channel 10.0 -InstallDir $HOME\\.dotnet-sdk\")")
    if dotnet:
        env["DOTNET"] = dotnet
    blender = find_blender()
    line(blender, f"Blender ({blender or 'not found'})", "Install Blender 4.2+ (tested with 5.2) from https://www.blender.org/download/ or set BLENDER=<blender.exe>")
    if blender:
        env["BLENDER"] = blender
        has = blender_has_psk(blender)
        line(has, "Blender extension io_scene_psk_psa",
             f"Run once:  \"{blender}\" --background --online-mode --command extension install --enable io_scene_psk_psa")
    natives = os.path.exists(TOOLS + "/ue-export/native/CUE4Parse-Natives.dll")
    vs = None
    if not natives:
        vsw = os.path.join(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)"), "Microsoft Visual Studio", "Installer", "vswhere.exe")
        if os.path.exists(vsw):
            r = subprocess.run([vsw, "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"], capture_output=True, text=True)
            vs = r.stdout.strip()
        line(vs, "Visual Studio C++ build tools (to build CUE4Parse-Natives.dll, animation decompression)",
             "Install 'Visual Studio Build Tools' with the 'Desktop development with C++' workload: https://visualstudio.microsoft.com/downloads/")
    else:
        line(True, "CUE4Parse-Natives.dll already built")
    line(os.path.exists(USMAP), f"Unreal mappings file {USMAP}",
         "This file cannot be made from the paks; it comes from the running game via UE4SS (once):\n"
         "  1. powershell -ExecutionPolicy Bypass -File tools\\ue4ss_probe\\install.ps1 -Game \"<Bodycam install dir>\"   (installs UE4SS into YOUR game folder)\n"
         "     or install UE4SS 'experimental' (UE 5.5 support) from https://github.com/UE4SS-RE/RE-UE4SS/releases yourself.\n"
         "  2. Start Bodycam, load into the lobby, then in the UE4SS window use the Dumpers tab > 'Generate .usmap file'\n"
         "     (default keybind Ctrl+Numpad6). It writes Mappings.usmap (or <name>.usmap) next to UE4SS.dll in Bodycam\\Binaries\\Win64.\n"
         f"  3. Copy that file to {USMAP}\n"
         "  4. Optionally remove UE4SS again:  install.ps1 -Uninstall. Then re-run setup.")
    return not bad, env


# ---------------------------------------------------------------- steps
ENV = dict(os.environ)
EXE = TOOLS + "/ue-export/bin/Release/net10.0/ue-export.dll"
REL = ROOT + "/target/release"
exe = lambda n: f"{REL}/{n}.exe" if os.name == "nt" else f"{REL}/{n}"
A = lambda p: f"{ASSETS}/{p}"


def sh(cmd, cwd=None, env=None, check=True):
    cmd = [str(c) for c in cmd]
    print("  $ " + " ".join(c if " " not in c else f'"{c}"' for c in cmd)[:300], flush=True)
    r = subprocess.run(cmd, cwd=cwd, env={**ENV, **(env or {})})
    if check and r.returncode:
        sys.exit(f"\nFAILED (exit {r.returncode}): {' '.join(cmd)[:300]}")


def py(script, *a, env=None):
    sh([sys.executable, "-I", f"{TOOLS}/{script}", *a], env=env)


def marker(path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w").write(time.strftime("%Y-%m-%d %H:%M:%S\n"))


# files that must exist for a step to count as done
def exists(*paths):
    return lambda: all(os.path.exists(p) for p in paths)


def s_tools():
    if not os.path.exists(TOOLS + "/ue-export/native/CUE4Parse-Natives.dll"):
        sh(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", TOOLS + "/ue-export/build-natives.ps1"])
    sh([ENV["DOTNET"], "build", "-c", "Release", TOOLS + "/ue-export"])
    sh(["cargo", "build", "--release", "-p", "extract", "-p", "uasset", "-p", "binka", "-p", "audio-bake"], cwd=ROOT)


# Everything the later steps read from the paks as loose cooked files (SoundWaves, cues, data tables, fonts, RC clip lists)
EXTRACT = ["Content/BodycamCore/ItemsDefinition/", "Content/Audio/", "Content/Ultimate_SFX/", "Content/FPV_Drone/Sfx/", "Content/RCCar/SFX/",
           "Content/RCCar/Animations/", "Content/BallisticsVFX/SFX/", "Content/AdvancedLocomotionV4/Audio/", "Content/AdvancedLocomotionV4/Sounds/",
           "Content/BodycamWeapons/LowPolyShooterPack/Audio/", "Sounds_Shell/", "BodycamAnimationFramework&/Sound", "Content/UI/Fonts/", "Content/UI/Sfx/"]


def s_extract():
    sh([exe("extract"), ENV["BODYCAM_GAME"] + "/Bodycam/Content/Paks", EXTRACTED, *EXTRACT])
    marker(EXTRACTED + "/.extract-done")


def s_dump():
    sh([exe("dump"), USMAP, EXTRACTED + "/Bodycam", EXTRACTED + "/json"])
    marker(EXTRACTED + "/json/.dump-done")


def s_fonts():
    src = EXTRACTED + "/Bodycam/Content/UI/Fonts"
    for dst, f in ((A("fonts/Ki-Bold.otf"), "ki/Ki-Bold"), (A("ui/Ki-ExtraBold.otf"), "ki/Ki-ExtraBold"), (A("ui/Ki-Medium.otf"), "ki/Ki-Medium"),
                   (A("ui/Oswald-SemiBold.otf"), "Oswald-SemiBold")):
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(f"{src}/{f}.ufont", dst)  # the .ufont files are the raw TTF/OTF data


# (input dir under extracted/Bodycam, output dir under assets-local/audio): binka keeps the path below the input dir
AUDIO = [("Content/Audio", ""), ("Content/Ultimate_SFX", "Ultimate_SFX"), ("Content/FPV_Drone/Sfx", "FPV_Drone/Sfx"), ("Content/RCCar/SFX", "RCCar/SFX"),
         ("Content/BallisticsVFX/SFX", "BallisticsVFX/SFX"), ("Content/AdvancedLocomotionV4/Sounds", "AdvancedLocomotionV4/Sounds"),
         ("Content/AdvancedLocomotionV4/Audio", "AdvancedLocomotionV4/Audio"),
         ("Content/BodycamWeapons/LowPolyShooterPack/Audio", "BodycamWeapons/LowPolyShooterPack/Audio"),
         ("Plugins/BodycamAnimationFramework/Content/Animations", "BodycamAnimationFramework/Animations"), ("Content/UI/Sfx", "UI/Sfx")]


def s_audio_wav():
    for src, dst in AUDIO:
        if os.path.isdir(f"{EXTRACTED}/Bodycam/{src}"):
            sh([exe("binka"), f"{EXTRACTED}/Bodycam/{src}", A("audio/" + dst).rstrip("/")])
    marker(A("audio/.done"))


def s_audio_bake():
    sh([exe("audio-bake"), EXTRACTED + "/json", ROOT + "/assets/weapons", ASSETS])


def s_ambients():
    py("ambients.py")


GUNS = "body,Akm,Benelli,Crossbow,Deagle,FAL,Glock17,M4A1,MP5,MP9,Mossberg590,Remington700,Revolver,SCAR-H,SVD,UZI,Vaiga,Vector,CZ-P-09,Kobra,M1911".split(",")
GUN_GLBS = [g for g in GUNS if g != "body"]


def s_models():
    py("build_models.py", "--only", ",".join(GUNS), "--resume")


ITEMS = "G67,Knife,Flashbang,N67,Karambit,Kamikaze,DroneController,DroneGoggles,Bomb".split(",")
EXTRAS = ["RGN", "MB78", "Flashbang02", "Folding", "Blade", "Sentinel"]


def s_items():
    work = A("models_items/_work")
    py("attachments.py", work)  # DT_PerkItems / DT_WeaponSkins -> <work>/attachments.json (receivers + parts for the item exports)
    py("build_items.py", ",".join(ITEMS + ["Tablet"]))
    py("build_extras.py")


def s_rccars():
    py("build_rccars.py")


def s_attachments():
    py("build_attachments.py")
    if not os.path.exists(A("models/attachments/ammo_8_gauge.glb")):
        py("build_ammo_shell.py")
    marker(A("models/attachments/.done"))


def s_textures():
    py("export_textures.py")
    py("export_textures.py", "genicons")
    marker(A("textures/.done"))


def s_map():
    os.makedirs(A("maps"), exist_ok=True)
    if not os.path.exists(A("maps/scene2.glb")) or not os.path.exists(A("maps/scene2.json")):
        sh([ENV["DOTNET"], EXE, ENV["BODYCAM_GAME"] + "/Bodycam/Content/Paks", USMAP, A("maps"), "scene", "Bodycam/Content/Map/Lobby/Lobby"],
           cwd=os.path.dirname(EXE), env={"SCENE_NAME": "scene2"})
    py("bake_bc.py", A("maps/scene2.glb"))  # BC1 + mips: ~6x less VRAM (keeps scene2.orig.glb)
    marker(A("maps/.baked"))


def s_vhs():
    py("vhs_atlas.py")


STEPS = [  # name, description, done-check, run
    ("tools", "build ue-export (.NET), CUE4Parse natives, and the Rust extract / dump / binka / audio-bake tools", exists(exe("extract"), exe("dump"), exe("binka"), exe("audio-bake"), EXE), s_tools),
    ("extract", "copy loose cooked files (audio, item tables, fonts) out of the paks", exists(EXTRACTED + "/.extract-done"), s_extract),
    ("dump", "dump the extracted .uasset files to JSON (needs Bodycam.usmap)", exists(EXTRACTED + "/json/.dump-done"), s_dump),
    ("fonts", "UI fonts -> assets-local/fonts + ui", exists(A("fonts/Ki-Bold.otf"), A("ui/Ki-ExtraBold.otf"), A("ui/Ki-Medium.otf"), A("ui/Oswald-SemiBold.otf")), s_fonts),
    ("audio-wav", "decode BINKA sounds to wav -> assets-local/audio", exists(A("audio/.done")), s_audio_wav),
    ("audio-bake", "bake SoundCue graphs / attenuation -> assets-local/audio-data/audio.json", exists(A("audio-data/audio.json")), s_audio_bake),
    ("ambients", "placed ambient sounds of the lobby -> assets-local/maps/ambients.json", exists(A("maps/ambients.json")), s_ambients),
    ("models", "arms, body, firearms + FP clips + camera tracks (UE -> PSK/PSA -> Blender -> glb)", exists(A("models/manifest.json"), A("models/body.glb"), *[A(f"models/guns/{g}.glb") for g in GUN_GLBS]), s_models),
    ("items", "grenades, knives, drones, bomb, tablet, shell + mesh-only items", exists(*[A(f"models/guns/{g}.glb") for g in ITEMS + EXTRAS], A("ui/Tablet.glb"), A("ui/arms_tablet.glb"), A("models/shell.glb")), s_items),
    ("rccars", "RC cars Trataxes / Kyoshin", exists(A("models/guns/Trataxes.glb"), A("models/guns/Kyoshin.glb")), s_rccars),
    ("attachments", "~960 attachment meshes + ammo meshes (long)", exists(A("models/attachments/.done")), s_attachments),
    ("textures", "fx sprites, decals, light cookies, lens overlays, UI art, icons, reticles", exists(A("textures/.done")), s_textures),
    ("map", "the lobby map -> assets-local/maps/scene2.glb + scene2.json (BC1 baked)", exists(A("maps/.baked")), s_map),
    ("vhs", "VHS movie -> death / loading screen atlas (downloads NihAV, builds bk2dump)", exists(A("textures/fx/vhs_01_atlas.ktx2")), s_vhs),
]

if args.list:
    for n, d, _, _ in STEPS:
        print(f"{n:12} {d}")
    sys.exit(0)

ok, env = prereqs()
if not ok:
    sys.exit("\nSetup cannot continue: fix the MISSING items above and run it again.")
ENV.update(env)
if args.check:
    print("\nAll prerequisites present.")
    sys.exit(0)

names = [s[0] for s in STEPS]
only = [x for x in args.only.split(",") if x] or names
skip = [x for x in args.skip.split(",") if x]
for x in only + skip:
    if x not in names:
        sys.exit(f"unknown step '{x}' (see --list)")
print(f"\n== game:      {ENV['BODYCAM_GAME']}\n== extracted: {EXTRACTED}\n== assets:    {ASSETS}\n")
t0 = time.time()
todo = [s for s in STEPS if s[0] in only and s[0] not in skip]
for i, (name, desc, done, run) in enumerate(todo, 1):
    head = f"[{i}/{len(todo)}] {name}: {desc}"
    if not args.force and name != "tools" and done():  # `tools` is an incremental build: always run it so the exes match the sources
        print(head + "  -- already done, skipping (use --force to redo)")
        continue
    print("\n" + head, flush=True)
    t = time.time()
    run()
    if not done():
        sys.exit(f"step '{name}' finished but its outputs are still missing; see the messages above")
    print(f"  done in {time.time() - t:.0f} s", flush=True)
print(f"\nSetup finished in {(time.time() - t0) / 60:.1f} min.\nPlay:  cargo run --release -p game" + (f"   (set BODYCAM_ASSETS={ASSETS} first)" if ASSETS != ROOT + "/assets-local" else ""))
