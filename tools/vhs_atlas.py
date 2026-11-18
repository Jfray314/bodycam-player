"""VHS movie (Movies/Vhs/Vhs_01.bk2, Bink 2; loose file in the game install) -> assets-local/textures/fx/vhs_01_atlas.{png,ktx2}.

  python -I tools/vhs_atlas.py        (needs git + cargo; downloads NihAV, GPL-3.0+, https://nihav.org, into the git-ignored tools/nihav)

NihAV decodes Bink 2; its bink demuxer is patched to accept the KB2n magic. tools/bk2dump writes every frame box-downscaled to 128x72 RGB,
the frames are packed as luma into a 64 x 90 grid of 128x72 cells (8192 x 6480 L8, frame i at column i % 64, row i // 64), then
tools/bake_vhs.py turns the PNG into the R8 KTX2 the game prefers. The vertical streak bands from frame ~550 on are real tape content.
"""
import os, subprocess, sys, tempfile, urllib.request
import numpy as np
from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GAME = os.environ.get("BODYCAM_GAME", r"C:\Program Files (x86)\Steam\steamapps\common\Bodycam")
ASSETS = os.environ.get("BODYCAM_ASSETS", ROOT + "/assets-local")
FX = ASSETS + "/textures/fx"
PNG, KTX = FX + "/vhs_01_atlas.png", FX + "/vhs_01_atlas.ktx2"
W, H, COLS = 128, 72, 64


def run(cmd, **kw):
    r = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if r.returncode:
        sys.exit("failed: " + " ".join(map(str, cmd))[:200] + "\n" + (r.stdout + r.stderr)[-800:])


nihav = ROOT + "/tools/nihav"
if not os.path.exists(PNG):
    bk2 = GAME + "/Bodycam/Content/Movies/Vhs/Vhs_01.bk2"
    if not os.path.exists(bk2):
        sys.exit("missing " + bk2)
    if not os.path.exists(nihav + "/nihav-core"):
        bundle = tempfile.mktemp(suffix=".gitbundle")
        print("  downloading NihAV (https://nihav.org/nihav.gitbundle)")
        urllib.request.urlretrieve("https://nihav.org/nihav.gitbundle", bundle)
        run(["git", "clone", "-q", bundle, nihav])
    p = nihav + "/nihav-rad/src/demuxers/bink.rs"
    s = open(p, encoding="utf8", newline="").read()
    if 'b"KB2n"' not in s:
        assert 'b"KB2k"' in s, "unexpected NihAV bink.rs"
        open(p, "w", encoding="utf8", newline="").write(s.replace('mktag!(b"KB2k")', 'mktag!(b"KB2n")'))
    print("  building bk2dump")
    run(["cargo", "build", "--release", "--manifest-path", ROOT + "/tools/bk2dump/Cargo.toml"])
    exe = ROOT + "/tools/bk2dump/target/release/bk2dump" + (".exe" if os.name == "nt" else "")
    rgb = tempfile.mktemp(suffix=".rgb")
    print("  decoding frames (a minute or two)")
    run([exe, bk2, rgb, "0", "100000", str(W), str(H)])
    fr = np.fromfile(rgb, np.uint8).reshape(-1, H, W, 3)
    os.remove(rgb)
    n = len(fr)
    rows = (n + COLS - 1) // COLS
    assert rows == 90 and n > 5600, f"unexpected frame count {n}"
    atlas = Image.new("L", (W * COLS, H * 90))
    for i in range(n):
        atlas.paste(Image.fromarray(fr[i]).convert("L"), ((i % COLS) * W, (i // COLS) * H))
    os.makedirs(FX, exist_ok=True)
    atlas.save(PNG)
    print("  frames:", n)
if not os.path.exists(KTX):
    run([sys.executable, "-I", ROOT + "/tools/bake_vhs.py", ASSETS])
