//! UE5 "Bink Audio 2" (UEBA, DCT variant) decoder, ported from FFmpeg's binkaudio.c/binka.c (LGPL-2.1+).
use std::f32::consts::PI;

pub struct Decoded {
    pub channels: usize,
    pub rate: u32,
    pub pcm: Vec<Vec<f32>>, // per channel
}

const CRIT: [u32; 25] = [
    100, 200, 300, 400, 510, 630, 770, 920, 1080, 1270, 1480, 1720, 2000, 2320, 2700, 3150, 3700, 4400,
    5300, 6400, 7700, 9500, 12000, 15500, 24500,
];
const RLE: [usize; 16] = [2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64];

struct Bits<'a>(&'a [u8], usize);
impl Bits<'_> {
    fn left(&self) -> usize { (self.0.len() * 8).saturating_sub(self.1) }
    fn get(&mut self, n: usize) -> u32 {
        let mut v = 0u64;
        for i in 0..n {
            let p = self.1 + i;
            let bit = self.0.get(p >> 3).map_or(0, |b| (b >> (p & 7)) & 1);
            v |= (bit as u64) << i;
        }
        self.1 += n;
        v as u32
    }
}

/// In-place radix-2 complex FFT (e^{+i...} kernel).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 { j ^= bit; bit >>= 1; }
        j |= bit;
        if i < j { re.swap(i, j); im.swap(i, j); }
    }
    let mut len = 2;
    while len <= n {
        let ang = 2.0 * std::f64::consts::PI / len as f64;
        for s in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((ang * k as f64).cos() as f32, (ang * k as f64).sin() as f32);
                let (a, b) = (s + k, s + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr; im[b] = im[a] - ti;
                re[a] += tr; im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// y[k] = scale * sum_n X[n] cos(pi/N (k+1/2) n), via a 2N-point FFT.
fn dct3(x: &[f32], scale: f32) -> Vec<f32> {
    let n = x.len();
    let (mut re, mut im) = (vec![0f32; 2 * n], vec![0f32; 2 * n]);
    for i in 0..n {
        let a = std::f64::consts::PI * i as f64 / (2 * n) as f64;
        re[i] = x[i] * a.cos() as f32;
        im[i] = x[i] * a.sin() as f32;
    }
    fft(&mut re, &mut im);
    let _ = PI;
    re[..n].iter().map(|v| v * scale).collect()
}

fn find(h: &[u8], pat: &[u8]) -> Option<usize> { h.windows(pat.len()).position(|w| w == pat) }
fn u32le(b: &[u8], o: usize) -> u32 { u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) }

/// Split the block stream into (packet data, optional sample-count override for the last packet).
fn packets(d: &[u8]) -> Option<Vec<(&[u8], Option<usize>)>> {
    let u16le = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    'start: for p in 0..d.len() {
        if d.get(p..p + 2) != Some(&[0x99, 0x99]) { continue; }
        let (mut o, mut out) = (p, vec![]);
        while d.len() - o > 4 {
            if d[o..o + 2] != [0x99, 0x99] { continue 'start; }
            let (mut sz, mut dur, mut hdr) = (u16le(o + 2).unwrap(), None, 4);
            if sz == 0xFFFF {
                match (u16le(o + 4), u16le(o + 6)) { (Some(s), Some(du)) => { sz = s; dur = Some(du); hdr = 8 } _ => continue 'start }
            }
            if sz == 0 || o + hdr + sz > d.len() { continue 'start; }
            out.push((&d[o + hdr..o + hdr + sz], dur));
            o += hdr + sz;
        }
        return Some(out);
    }
    None
}

/// Decode a UEBA sound: `uexp` holds the header, `bulk` is the .ubulk (None => stream follows header in uexp).
pub fn decode(uexp: &[u8], bulk: Option<&[u8]>) -> Result<Decoded, String> {
    let h = find(uexp, b"ABEU").ok_or("no UEBA header")?;
    let (ch, rate, count) = (uexp[h + 5] as usize, u32le(uexp, h + 8), u32le(uexp, h + 12) as usize);
    if ch == 0 || ch > 2 || uexp[h + 4] != 1 { return Err(format!("unsupported UEBA ch={ch} ver={}", uexp[h + 4])); }
    let data = bulk.unwrap_or(&uexp[h + 0x1a..]);
    let pk = packets(data).ok_or("no block stream")?;

    let fl: usize = if rate < 22050 { 512 } else if rate < 44100 { 1024 } else { 2048 };
    let ov = fl / 16;
    let half = (rate + 1) / 2;
    let root = fl as f32 / ((fl as f32).sqrt() * 32768.0);
    let quant: Vec<f32> = (0..96).map(|i| (i as f32 * 0.152_891_65).exp() * root).collect();
    let mut nb = 1;
    while nb < 25 && half > CRIT[nb - 1] { nb += 1; }
    let mut bands = vec![2usize];
    for i in 1..nb { bands.push(((CRIT[i - 1] as usize * fl) / half as usize) & !1); }
    bands.push(fl);

    let mut prev = vec![vec![0f32; ov]; ch];
    let mut pcm = vec![Vec::<f32>::new(); ch];
    let mut first = true;
    for (data, dur) in pk {
        let mut b = Bits(data, 0);
        b.get(2);
        let mut outs = vec![];
        for _ in 0..ch {
            if b.left() < 58 + nb * 7 { return Err("truncated block".into()); }
            let gf = |b: &mut Bits| {
                let p = b.get(5) as i32;
                let m = b.get(23) as f32;
                let f = m * 2f32.powi(p - 23);
                if b.get(1) == 1 { -f } else { f }
            };
            let mut c = vec![0f32; fl];
            c[0] = gf(&mut b) * root;
            c[1] = gf(&mut b) * root;
            let q: Vec<f32> = (0..nb).map(|_| quant[(b.get(7) as usize).min(95)]).collect();
            let (mut k, mut qq, mut i) = (0usize, q[0], 2usize);
            while i < fl {
                let j = if b.get(1) == 1 { i + RLE[b.get(4) as usize] * 8 } else { i + 8 };
                let j = j.min(fl);
                let w = b.get(4) as usize;
                if w == 0 {
                    i = j;
                    while bands[k] < i { qq = q[k]; k += 1; }
                } else {
                    for m in i..j { c[m] = b.get(w) as f32; }
                    while i < j {
                        if bands[k] == i { qq = q[k]; k += 1; }
                        if c[i] > 0.0 { c[i] *= if b.get(1) == 1 { -qq } else { qq }; }
                        i += 1;
                    }
                }
            }
            c[0] /= 0.5;
            outs.push(dct3(&c, 1.0 / fl as f32));
        }
        for c in 0..ch {
            let o = &mut outs[c];
            let count = (ov * ch) as f32;
            if !first {
                for i in 0..ov {
                    let j = (c + i * ch) as f32;
                    o[i] = (prev[c][i] * (count - j) + o[i] * j) / count;
                }
            }
            prev[c].copy_from_slice(&o[fl - ov..]);
            let n = (fl - ov).min(dur.unwrap_or(usize::MAX));
            pcm[c].extend_from_slice(&o[..n]);
        }
        first = false;
    }
    for p in &mut pcm { p.truncate(count); }
    Ok(Decoded { channels: ch, rate, pcm })
}

/// Codec name from the .uasset name table.
pub fn codec_name(uasset: &[u8]) -> &'static str {
    for n in ["BINKA", "OPUS", "ADPCM", "OGG", "VORBIS", "PCM", "Float"] {
        if find(uasset, n.as_bytes()).is_some() { return n; }
    }
    "none"
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_pistol() {
        let b = "C:/code/bodycam-player/extracted/Bodycam/Content/Audio/Guns/Wav/Pistol/Punch/CLEAN/Bodycam_Guns_Pistol_Punch_Outside_Close_CLEAN_v2_001";
        let (Ok(e), Ok(k)) = (std::fs::read(format!("{b}.uexp")), std::fs::read(format!("{b}.ubulk"))) else { return };
        let d = super::decode(&e, Some(&k)).unwrap();
        assert_eq!((d.channels, d.rate, d.pcm[0].len()), (2, 48000, 24432));
        let all: Vec<f32> = d.pcm.iter().flatten().copied().collect();
        let rms = (all.iter().map(|x| x * x).sum::<f32>() / all.len() as f32).sqrt();
        let peak = all.iter().fold(0f32, |a, x| a.max(x.abs()));
        assert!(all.iter().all(|x| x.is_finite()));
        assert!(rms > 0.005 && rms < 0.5 && peak > 0.1 && peak < 4.0, "rms {rms} peak {peak}");
    }
}
