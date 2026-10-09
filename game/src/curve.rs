//! UE FRichCurve evaluation. Keys are [time, value, interp_mode (0 linear, 1 constant, 2 cubic), arrive, leave]
//! as baked by tools/weapon_data.py.

pub type Key = [f32; 5];

pub fn eval(keys: &[Key], t: f32) -> f32 {
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return 0.0 };
    if t <= first[0] {
        return first[1];
    }
    if t >= last[0] {
        return last[1];
    }
    let i = keys.partition_point(|k| k[0] <= t) - 1;
    let (a, b) = (keys[i], keys[i + 1]);
    let dt = b[0] - a[0];
    let s = (t - a[0]) / dt;
    match a[2] as i32 {
        0 => a[1] + (b[1] - a[1]) * s,
        1 => a[1],
        _ => {
            // Cubic Hermite with UE's tangents (value per second, scaled by the segment length).
            let (s2, s3) = (s * s, s * s * s);
            let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
            let h10 = s3 - 2.0 * s2 + s;
            let h01 = -2.0 * s3 + 3.0 * s2;
            let h11 = s3 - s2;
            h00 * a[1] + h10 * dt * a[4] + h01 * b[1] + h11 * dt * b[3]
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hits_keys_and_clamps() {
        let k = [[0.0, 0.0, 2.0, 0.0, 0.0], [0.5, 1.0, 2.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0, 0.0]];
        assert_eq!(super::eval(&k, -1.0), 0.0);
        assert!((super::eval(&k, 0.5) - 1.0).abs() < 1e-6);
        assert!((super::eval(&k, 0.25) - 0.5).abs() < 1e-6); // flat tangents -> smoothstep midpoint
        assert_eq!(super::eval(&k, 2.0), 0.0);
    }
}
