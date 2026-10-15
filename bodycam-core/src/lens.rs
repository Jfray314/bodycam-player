//! Lens ("bodycam look") parameters shared by the game post-effect (game/src/postfx.rs), the ReShade file
//! mods/reshade/Bodycam.fx, and any host-side implementation; plus the angular-rate smoothing and the exposure-pump spring
//! that drive the shader's `yaw_rate`/`pitch_rate`/`pump` inputs.
//! Presets differ only where ReShade sees display-referred 0..1 colour instead of HDR (dirt threshold/gain).

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct LensParams {
    pub distortion: f32,
    pub distortion_density: f32,
    pub ca: f32,
    pub vignette: f32,
    pub aberration_fade_exponent: f32,
    pub edge_blur: f32,
    pub blur_fade_exponent: f32,
    pub sharpen: f32,
    pub grain: f32,
    pub temporal_noise: f32,
    pub banding: f32,
    pub dirt: f32,
    pub dirt_threshold: f32,
    pub dirt_gain: f32,
    pub exposure_bias: f32,
    /// Sensor readout time in seconds (game only; ReShade cannot do it).
    pub rolling_shutter: f32,
}

impl LensParams {
    /// Defaults of the game's BodycamPostFx.
    pub const fn game() -> Self {
        LensParams {
            distortion: 0.33, distortion_density: 1.0, ca: 0.003, vignette: 0.4, aberration_fade_exponent: 2.0, edge_blur: 25.0 * 0.8,
            blur_fade_exponent: 10.0, sharpen: 0.0, grain: 0.0, temporal_noise: 0.0, banding: 0.0, dirt: 0.25, dirt_threshold: 4.0,
            dirt_gain: 0.6, exposure_bias: 0.0, rolling_shutter: 0.010,
        }
    }
    /// Defaults of mods/reshade/Bodycam.fx (checked against the file by a test).
    pub const fn reshade() -> Self {
        LensParams { dirt_threshold: 0.8, dirt_gain: 1.5, rolling_shutter: 0.0, ..Self::game() }
    }
}

/// Horizontal FOV (UE FieldOfView is horizontal; Bodycam keeps it at 120 also while aiming) -> vertical FOV in degrees.
pub fn vertical_fov_deg(h_fov_deg: f32, aspect: f32) -> f32 {
    (2.0 * ((h_fov_deg.to_radians() * 0.5).tan() / aspect).atan()).to_degrees()
}

/// Smoothed angular rates and the exposure-pump spring (port of postfx.rs `motion_state`).
#[derive(Default, Clone, Copy, Debug)]
pub struct LensMotion {
    pub yaw_rate: f32,
    pub pitch_rate: f32,
    pub pump: f32,
    vel: f32,
}

impl LensMotion {
    /// `w` = camera angular velocity in camera-local axes (x pitch, y yaw, z roll), rad/s.
    pub fn step(&mut self, dt: f32, w: [f32; 3]) {
        let dt = dt.max(1e-4);
        let k = (dt * 12.0).min(1.0);
        self.yaw_rate += (w[1].clamp(-12.0, 12.0) - self.yaw_rate) * k;
        self.pitch_rate += (w[0].clamp(-12.0, 12.0) - self.pitch_rate) * k;
        let kick = ((w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt() - 2.0).max(0.0) * 0.02;
        self.vel += (-18.0 * self.pump - 3.0 * self.vel - kick * 60.0) * dt;
        self.pump += self.vel * dt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default of `uniform float <name> ... = <v>;` in the ReShade file.
    fn fx(src: &str, name: &str) -> f32 {
        let at = src.find(&format!("uniform float {name} ")).unwrap_or_else(|| panic!("{name}"));
        let rest = src[at..].split_once("> =").unwrap().1; // declarations may span lines
        rest.split(';').next().unwrap().trim().parse().unwrap()
    }

    #[test]
    fn reshade_preset_matches_the_fx_file() {
        let src = include_str!("../../mods/reshade/Bodycam.fx");
        let p = LensParams::reshade();
        for (n, v) in [
            ("distortion", p.distortion), ("distortion_density", p.distortion_density), ("ca", p.ca), ("aberration_fade_exponent", p.aberration_fade_exponent),
            ("edge_blur", p.edge_blur), ("blur_fade_exponent", p.blur_fade_exponent), ("sharpen", p.sharpen), ("dirt", p.dirt), ("dirt_threshold", p.dirt_threshold),
            ("dirt_gain", p.dirt_gain), ("exposure_bias", p.exposure_bias), ("vignette", p.vignette), ("grain", p.grain), ("temporal_noise", p.temporal_noise), ("banding", p.banding),
        ] {
            assert!((fx(src, n) - v).abs() < 1e-6, "{n}: fx {} vs core {v}", fx(src, n));
        }
    }

    #[test]
    fn fov_and_pump() {
        assert!((vertical_fov_deg(90.0, 1.0) - 90.0).abs() < 1e-3);
        assert!((vertical_fov_deg(120.0, 16.0 / 9.0) - 88.6).abs() < 0.2);
        let mut m = LensMotion::default();
        for _ in 0..30 {
            m.step(1.0 / 60.0, [0.0, 8.0, 0.0]); // fast turn kicks the pump
        }
        assert!(m.pump < 0.0 && m.yaw_rate > 4.0);
        for _ in 0..600 {
            m.step(1.0 / 60.0, [0.0; 3]);
        }
        assert!(m.pump.abs() < 1e-3 && m.yaw_rate.abs() < 1e-3);
    }
}
