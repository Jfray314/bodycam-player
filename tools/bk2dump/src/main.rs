// bk2dump <in.bk2> <out.rgb> <first> <count> <out_w> <out_h>: decodes Bink 2 video frames with NihAV and writes them
// box-downscaled to raw RGB8 (count * out_w * out_h * 3 bytes).
use nihav_core::codecs::*;
use nihav_core::demuxers::*;
use nihav_core::io::byteio::FileReader;
use std::io::Write;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (first, count, ow, oh): (usize, usize, usize, usize) = (a[3].parse().unwrap(), a[4].parse().unwrap(), a[5].parse().unwrap(), a[6].parse().unwrap());
    let mut dmx_reg = RegisteredDemuxers::new();
    nihav_rad::rad_register_all_demuxers(&mut dmx_reg);
    let mut dec_reg = RegisteredDecoders::new();
    nihav_rad::rad_register_all_decoders(&mut dec_reg);
    let mut file = std::fs::File::open(&a[1]).unwrap();
    let mut br = FileReader::new_read(&mut file);
    let mut dmx = create_demuxer(dmx_reg.find_demuxer("bink").unwrap(), &mut br).unwrap();
    let mut vid = None;
    for i in 0..dmx.get_num_streams() {
        let s = dmx.get_stream(i).unwrap();
        let info = s.get_info();
        if info.is_video() {
            let mut dec = (dec_reg.find_decoder(info.get_name()).unwrap())();
            let mut sup = NADecoderSupport::new();
            dec.init(&mut sup, info).unwrap();
            vid = Some((s.get_id(), sup, dec));
        }
    }
    let (vid_id, mut sup, mut dec) = vid.unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&a[2]).unwrap());
    let (mut n, mut written) = (0usize, 0usize);
    while written < count {
        let pkt = match dmx.get_frame() {
            Ok(p) => p,
            Err(_) => break,
        };
        if pkt.get_stream().get_id() != vid_id {
            continue;
        }
        let frm = dec.decode(&mut sup, &pkt).unwrap();
        n += 1;
        if n <= first {
            continue;
        }
        let NABufferType::Video(buf) = frm.get_buffer() else { panic!("not 8-bit video") };
        let (w, h) = buf.get_dimensions(0);
        let d = buf.get_data();
        let (oy, ou, ov) = (buf.get_offset(0), buf.get_offset(1), buf.get_offset(2));
        let (sy, su, sv) = (buf.get_stride(0), buf.get_stride(1), buf.get_stride(2));
        let mut img = vec![0u8; ow * oh * 3];
        for y in 0..oh {
            for x in 0..ow {
                let (x0, x1, y0, y1) = (x * w / ow, (x + 1) * w / ow, y * h / oh, (y + 1) * h / oh);
                let (mut acc, mut k) = ([0f32; 3], 0f32);
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        let l = d[oy + yy * sy + xx] as f32;
                        let u = d[ou + (yy / 2) * su + xx / 2] as f32 - 128.0;
                        let v = d[ov + (yy / 2) * sv + xx / 2] as f32 - 128.0;
                        acc[0] += l + 1.402 * v;
                        acc[1] += l - 0.344 * u - 0.714 * v;
                        acc[2] += l + 1.772 * u;
                        k += 1.0;
                    }
                }
                for c in 0..3 {
                    img[(y * ow + x) * 3 + c] = (acc[c] / k).clamp(0.0, 255.0) as u8;
                }
            }
        }
        out.write_all(&img).unwrap();
        written += 1;
    }
    eprintln!("decoded {n}, wrote {written}");
}
