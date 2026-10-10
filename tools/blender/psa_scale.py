"""Pure-python PSK/PSA reader: detect clips whose translations are not in the PSK's units.
Some skeletons (e.g. Remington700: SKEL_RM700 root) have PSA translations exactly 100x the PSK ref pose
(UE skeleton ref pose carries a 0.01 root scale), which explodes the mesh when imported with the usual cm->m 0.01.
python psa_scale.py <mesh.psk> <clip.psa> -> prints the translation scale factor to multiply on top of 0.01."""
import struct, math, sys


def _chunks(path):
    b = open(path, "rb").read(); i = 0; r = {}
    while i < len(b):
        n = b[i:i + 20].split(b"\0")[0].decode(); sz, c = struct.unpack_from("<ii", b, i + 24)
        r[n] = (b, i + 32, sz, c); i += 32 + sz * c
    return r


def _names(b, o, sz, c):
    return [b[o + k * sz:o + k * sz + 64].split(b"\0")[0].decode() for k in range(c)]


def trans_ratio(psk, psa):
    """median |PSA first-frame translation| / |PSK ref translation| over bones with ref length > 1 cm, snapped to a power of 10."""
    b, o, sz, c = _chunks(psk)["REFSKELT"]
    ref = {n: struct.unpack_from("<3f", b, o + k * sz + 64 + 28) for k, n in enumerate(_names(b, o, sz, c))}
    ch = _chunks(psa)
    nm = _names(*ch["BONENAMES"])
    b, o, sz, c = ch["ANIMKEYS"]
    L = lambda v: math.sqrt(sum(x * x for x in v))
    rs = sorted(L(struct.unpack_from("<3f", b, o + k * sz)) / L(ref[n]) for k, n in enumerate(nm) if n in ref and L(ref[n]) > 1)
    if not rs:
        return 1.0
    return 10.0 ** round(math.log10(max(rs[len(rs) // 2], 1e-9)))


if __name__ == "__main__":
    print(trans_ratio(sys.argv[1], sys.argv[2]))
