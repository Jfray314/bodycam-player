//! Engine-free port of the Bodycam player's camera motion (game/src/player.rs, curve.rs): idle pose, gait loops,
//! land event, lean, spring roll, smooth control rotation lag, controller recoil, fire shake.
//! No Bevy. Unreal conventions on the public side: metres (x right, y up, z back), degrees (pitch up+, yaw right+, roll cw+).
//! Internally the maths is the game's (measured data is "game convention": yaw left+, roll ccw+); `step` converts.
use serde::Deserialize;

const WALK: f32 = 1.25;
const SPRINT: f32 = 3.25;
const CROUCH: f32 = 1.00;
const ADS: f32 = 1.00;
const HALF_HEIGHT: f32 = 0.90;
const CROUCH_HALF_HEIGHT: f32 = 0.55;
const LEAN_SPEED: f32 = 12.0;
const LEAN_POS_SPEED: f32 = 8.0;
const LEAN_ROLL_SPEED: f32 = 2.0;
const YAW_DELTA_PER_DEG_S: f32 = 0.00796;
const ROLL_SPEED: f32 = 1.77;
const ROLL_LOOK_GAIN: f32 = 2.59;
const CTL_INPUT_SCALE: f32 = 2.5;
const ONE_RAD_DEG: f32 = 57.29578;

pub const GROUNDED: u32 = 1;
pub const CROUCHED: u32 = 1 << 1;
pub const SPRINTING: u32 = 1 << 2;
pub const AIMING: u32 = 1 << 3;
pub const TRIGGER_HELD: u32 = 1 << 4;
pub const LEAN_PRESSED: u32 = 1 << 5;
pub const LANDED: u32 = 1 << 6;

/// UE FInterpTo.
pub fn finterp_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let d = target - current;
    if d * d < 1e-8 {
        return target;
    }
    current + d * (dt * speed).clamp(0.0, 1.0)
}

/// Spring Camera roll target (UE deg, cw +).
pub fn spring_roll_target(move_x: f32, yaw_delta: f32) -> f32 {
    let map = |v: f32, a: f32, b: f32, c: f32, d: f32| c + (d - c) * ((v - a) / (b - a)).clamp(0.0, 1.0);
    map(move_x, -1.0, 1.0, -5.0, 5.0) + ROLL_LOOK_GAIN * map(yaw_delta, -3.0, 3.0, 8.0, -8.0)
}

// ---- UE FRichCurve (game/src/curve.rs) ----
pub(crate) type Key = [f32; 5];

pub(crate) fn curve_eval(keys: &[Key], t: f32) -> f32 {
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
            let (s2, s3) = (s * s, s * s * s);
            let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
            let h10 = s3 - 2.0 * s2 + s;
            let h01 = -2.0 * s3 + 3.0 * s2;
            let h11 = s3 - s2;
            h00 * a[1] + h10 * dt * a[4] + h01 * b[1] + h11 * dt * b[3]
        }
    }
}

// ---- data (assets/camera_motion.json, assets/fire_shake.json: our own measurements, not game assets) ----
#[derive(Deserialize)]
struct Loop {
    period_s: f32,
    curve: Vec<[f32; 6]>,
}
#[derive(Deserialize)]
struct Event {
    pre_s: f32,
    series: Vec<[f32; 6]>,
}
#[derive(Deserialize)]
struct Seq {
    series: Vec<[f32; 6]>,
}
#[derive(Deserialize)]
struct Loops {
    walk: Loop,
    sprint: Loop,
    crouch_walk: Loop,
    ads_walk: Loop,
}
#[derive(Deserialize)]
struct Events {
    land: Event,
}
#[derive(Deserialize)]
struct Seqs {
    lean: Seq,
}
#[derive(Deserialize)]
struct Motion {
    hz: f32,
    idle: [f32; 6],
    crouch_idle: [f32; 6],
    loops: Loops,
    events: Events,
    sequences: Seqs,
}
#[derive(Deserialize)]
struct Range2 {
    x: f32,
    y: f32,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ShakeCurves {
    float_curves: Vec<Vec<Key>>,
}
#[derive(Deserialize)]
struct ShakeCurve {
    curves: ShakeCurves,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FireShake {
    #[serde(rename = "pitch")]
    pitch: Range2,
    yaw: Range2,
    roll: Range2,
    smoothing: f32,
    play_rate: f32,
    rotation_curve: ShakeCurve,
}

fn sample_loop(l: &Loop, phase: f32) -> [f32; 6] {
    let n = l.curve.len();
    let f = phase.rem_euclid(1.0) * n as f32;
    let (i, a) = (f as usize % n, f.fract());
    let (p, q) = (l.curve[i], l.curve[(i + 1) % n]);
    std::array::from_fn(|k| p[k] + (q[k] - p[k]) * a)
}

fn add(a: &mut [f32; 6], b: [f32; 6], w: f32) {
    for k in 0..6 {
        a[k] += b[k] * w;
    }
}

/// Per-shot controller recoil parameters (PRAS RecoilData.ControllerRecoil), degrees.
#[derive(Clone, Copy, Debug)]
pub struct ControllerRecoil {
    pub horizontal: (f32, f32),
    pub vertical: (f32, f32),
    pub smoothing: f32,
    pub damping: f32,
    pub compensation: f32,
}

#[derive(Default)]
struct CtlRecoil {
    pending: [f32; 2],
    offset: [f32; 2],
    smoothing: f32,
    damping: f32,
    comp: f32,
}

impl CtlRecoil {
    fn tick(&mut self, dt: f32, input: [f32; 2], trigger_held: bool) -> [f32; 2] {
        let k = (self.smoothing * dt).min(1.0);
        let mut out = [0.0; 2];
        for i in 0..2 {
            let step = self.pending[i] * k;
            self.pending[i] -= step;
            self.offset[i] += step;
            if input[i] * self.offset[i] < 0.0 {
                let c = (input[i].abs() * self.comp).min(self.offset[i].abs());
                self.offset[i] -= c * self.offset[i].signum();
            }
            let ret = if trigger_held { 0.0 } else { self.offset[i] * (self.damping * dt).min(1.0) };
            self.offset[i] -= ret;
            out[i] = step - ret;
        }
        out
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Input {
    pub dt: f32,
    pub look_yaw_deg: f32,
    pub look_pitch_deg: f32,
    pub move_x: f32,
    pub speed: f32,
    pub lean_target: f32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct BcInput {
    pub dt: f32,
    pub look_yaw_deg: f32,
    pub look_pitch_deg: f32,
    pub move_x: f32,
    pub speed: f32,
    pub lean_target: f32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct BcOutput {
    pub pos: [f32; 3],
    pub rot: [f32; 3],
    pub base_pos: [f32; 3],
    pub base_rot: [f32; 3],
    pub ctl_yaw_deg: f32,
    pub ctl_pitch_deg: f32,
    pub ctl_delta_yaw_deg: f32,
    pub ctl_delta_pitch_deg: f32,
    pub lean: f32,
    pub footstep: i32,
    pub stride_start: i32,
}

pub struct Core {
    cm: Motion,
    shake: FireShake,
    rng: u64,
    // control rotation, UE convention (yaw right+, pitch up+), degrees
    yaw: f32,
    pitch: f32,
    smooth_yaw: f32,
    smooth_pitch: f32,
    yaw_rate: f32,
    lean: f32,
    lean_pos: f32,
    lean_roll: f32,
    center: f32,
    phase: f32,
    since_land: f32,
    roll: f32,
    ctl: CtlRecoil,
    pub shakes: Vec<(f32, [f32; 3])>,
    shake_out: [f32; 3],
}

impl Core {
    pub fn new(seed: u64) -> Self {
        Core {
            cm: serde_json::from_str(include_str!("../../assets/camera_motion.json")).expect("camera_motion.json"),
            shake: serde_json::from_str(include_str!("../../assets/fire_shake.json")).expect("fire_shake.json"),
            rng: seed | 1,
            yaw: 0.0,
            pitch: 0.0,
            smooth_yaw: 0.0,
            smooth_pitch: 0.0,
            yaw_rate: 0.0,
            lean: 0.0,
            lean_pos: 0.0,
            lean_roll: 0.0,
            center: HALF_HEIGHT,
            phase: 0.0,
            since_land: 99.0,
            roll: 0.0,
            ctl: CtlRecoil::default(),
            shakes: Vec::new(),
            shake_out: [0.0; 3],
        }
    }

    pub fn set_control(&mut self, yaw: f32, pitch: f32) {
        (self.yaw, self.pitch, self.smooth_yaw, self.smooth_pitch) = (yaw, pitch, yaw, pitch);
    }

    fn rand01(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        ((self.rng.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32) / 16777216.0
    }

    /// One shot; `r` = five uniform [0,1) draws (recoil h, recoil v, shake pitch, yaw, roll).
    pub fn fire_with(&mut self, c: &ControllerRecoil, r: [f32; 5]) {
        let pick = |(a, b): (f32, f32), u: f32| if a == b { a } else { a.min(b) + (a.max(b) - a.min(b)) * u };
        self.ctl.pending[0] += pick(c.horizontal, r[0]) * CTL_INPUT_SCALE;
        self.ctl.pending[1] += pick(c.vertical, r[1]) * CTL_INPUT_SCALE;
        self.ctl.smoothing = c.smoothing;
        (self.ctl.damping, self.ctl.comp) = (c.damping, c.compensation);
        let s = &self.shake;
        let amp = [pick((s.pitch.x, s.pitch.y), r[2]), pick((s.yaw.x, s.yaw.y), r[3]), pick((s.roll.x, s.roll.y), r[4])];
        self.shakes.push((0.0, amp));
    }

    pub fn fire(&mut self, c: &ControllerRecoil) {
        let r = std::array::from_fn(|_| self.rand01());
        self.fire_with(c, r);
    }

    fn fire_shake(&mut self, dt: f32) -> [f32; 3] {
        let s = &self.shake;
        let c = &s.rotation_curve.curves.float_curves;
        let mut target = [0.0f32; 3];
        for (age, amp) in &mut self.shakes {
            *age += dt;
            let t = *age * s.play_rate;
            for k in 0..3 {
                target[k] += curve_eval(&c[k], t) * amp[k];
            }
        }
        let end = c.iter().filter_map(|k| k.last()).map(|k| k[0]).fold(0.0, f32::max) / s.play_rate;
        self.shakes.retain(|(age, _)| *age < end);
        let k = (s.smoothing * dt).min(1.0);
        for i in 0..3 {
            self.shake_out[i] += (target[i] - self.shake_out[i]) * k;
        }
        self.shake_out
    }

    fn lean_pose(&self) -> [f32; 6] {
        *self.cm.sequences.lean.series.iter().max_by(|a, b| a[5].abs().total_cmp(&b[5].abs())).unwrap()
    }

    pub fn step(&mut self, inp: &Input) -> BcOutput {
        let dt = inp.dt;
        let f = |b: u32| inp.flags & b != 0;
        let (crouched, sprinting, aiming, grounded) = (f(CROUCHED), f(SPRINTING), f(AIMING), f(GROUNDED));
        let mut out = BcOutput::default();

        // look (+ controller recoil)
        let step = self.ctl.tick(dt, [inp.look_yaw_deg, inp.look_pitch_deg], f(TRIGGER_HELD));
        let (y0, p0) = (self.yaw, self.pitch);
        self.yaw += inp.look_yaw_deg + step[0];
        self.pitch = (self.pitch + inp.look_pitch_deg + step[1]).clamp(-89.9, 89.9);
        self.yaw_rate = inp.look_yaw_deg / dt.max(1e-4) * YAW_DELTA_PER_DEG_S;
        out.ctl_delta_yaw_deg = self.yaw - y0;
        out.ctl_delta_pitch_deg = self.pitch - p0;

        // lean + land timers (movement / lean systems)
        if f(LANDED) {
            self.since_land = 0.0;
            out.footstep = 4;
        }
        self.since_land += dt;
        self.lean = finterp_to(self.lean, inp.lean_target, dt, LEAN_SPEED);

        // bodycam_camera
        let target_center = if crouched { CROUCH_HALF_HEIGHT } else { HALF_HEIGHT };
        self.center += (target_center - self.center) * (12.0 * dt).min(1.0);
        let (gait, ref_speed) = if crouched {
            (&self.cm.loops.crouch_walk, CROUCH)
        } else if sprinting {
            (&self.cm.loops.sprint, SPRINT)
        } else if aiming {
            (&self.cm.loops.ads_walk, ADS)
        } else {
            (&self.cm.loops.walk, WALK)
        };
        let w = if grounded { (inp.speed / ref_speed).min(1.0) } else { 0.0 };
        let before = self.phase;
        self.phase += dt / gait.period_s * w;
        if w > 0.3 && (self.phase * 2.0).floor() > (before * 2.0).floor() {
            out.footstep = if crouched { 1 } else if sprinting { 3 } else { 2 };
            out.stride_start = (self.phase.floor() > before.floor()) as i32;
        }
        let crouch_a = ((HALF_HEIGHT - self.center) / (HALF_HEIGHT - CROUCH_HALF_HEIGHT)).clamp(0.0, 1.0);
        let cm = &self.cm;
        let mut c: [f32; 6] = std::array::from_fn(|k| cm.idle[k] + (cm.crouch_idle[k] - cm.idle[k]) * crouch_a);
        add(&mut c, sample_loop(gait, self.phase), w);
        let land = &cm.events.land;
        let i = ((self.since_land + land.pre_s) * cm.hz) as usize;
        if self.since_land < 5.0 && i < land.series.len() {
            add(&mut c, land.series[i], 1.0);
        }
        let lp = self.lean_pose();
        let side = if lp[5] < 0.0 { 1.0 } else { -1.0 };
        self.lean_pos = finterp_to(self.lean_pos, self.lean, dt, LEAN_POS_SPEED);
        self.lean_roll = finterp_to(self.lean_roll, self.lean_pos, dt, LEAN_ROLL_SPEED);
        add(&mut c, [lp[0], lp[1], lp[2], 0.0, 0.0, 0.0].map(|v| v * side), self.lean_pos);
        add(&mut c, [0.0, 0.0, 0.0, lp[3], lp[4], lp[5]].map(|v| v * side), self.lean_roll);

        if (self.yaw - self.smooth_yaw).abs() > ONE_RAD_DEG {
            self.smooth_yaw = self.yaw;
        }
        self.smooth_yaw = finterp_to(self.smooth_yaw, self.yaw, dt, 25.0);
        self.smooth_pitch = finterp_to(self.smooth_pitch, self.pitch, dt, 25.0);
        let lag_yaw = self.smooth_yaw - self.yaw; // UE sense (right +)
        let lag_pitch = self.smooth_pitch - self.pitch;
        let roll_target = if aiming || f(LEAN_PRESSED) { 0.0 } else { spring_roll_target(inp.move_x, self.yaw_rate) };
        self.roll = finterp_to(self.roll, roll_target, dt, ROLL_SPEED);
        c[5] -= self.roll;

        // measured data is yaw left+, roll ccw+: convert to UE (yaw right+, roll cw+).
        let center = self.center;
        let to_ue = |c: &[f32; 6]| ([c[0], center + c[1], c[2]], [lag_pitch + c[3], lag_yaw - c[4], -c[5]]);
        (out.base_pos, out.base_rot) = to_ue(&c);
        let fs = self.fire_shake(dt); // UE convention already
        add(&mut c, [0.0, 0.0, 0.0, fs[0], -fs[1], -fs[2]], 1.0);
        (out.pos, out.rot) = to_ue(&c);
        out.ctl_yaw_deg = self.yaw;
        out.ctl_pitch_deg = self.pitch;
        out.lean = self.lean;
        out
    }
}

// ---- C ABI ----
#[unsafe(no_mangle)]
pub extern "C" fn bc_create(seed: u64) -> *mut Core {
    std::panic::catch_unwind(|| Box::into_raw(Box::new(Core::new(seed)))).unwrap_or(std::ptr::null_mut())
}

/// # Safety
/// `c` must come from `bc_create` and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_destroy(c: *mut Core) {
    if !c.is_null() {
        drop(unsafe { Box::from_raw(c) });
    }
}

/// # Safety
/// `c` must be a live pointer from `bc_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_set_control(c: *mut Core, yaw: f32, pitch: f32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.set_control(yaw, pitch);
    }
}

/// # Safety
/// `c` must be a live pointer from `bc_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_fire(c: *mut Core, h_min: f32, h_max: f32, v_min: f32, v_max: f32, smoothing: f32, damping: f32, compensation: f32) {
    if let Some(c) = unsafe { c.as_mut() } {
        c.fire(&ControllerRecoil { horizontal: (h_min, h_max), vertical: (v_min, v_max), smoothing, damping, compensation });
    }
}

/// # Safety
/// `c` live from `bc_create`; `inp` and `out` valid for reads/writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_step(c: *mut Core, inp: *const BcInput, out: *mut BcOutput) {
    let (Some(c), Some(i), false) = (unsafe { c.as_mut() }, unsafe { inp.as_ref() }, out.is_null()) else { return };
    let input = Input { dt: i.dt, look_yaw_deg: i.look_yaw_deg, look_pitch_deg: i.look_pitch_deg, move_x: i.move_x, speed: i.speed, lean_target: i.lean_target, flags: i.flags };
    unsafe { *out = c.step(&input) };
}

pub mod ballistics;
pub mod cues;
pub mod ffi;
pub mod gunfire;
pub mod lens;
pub mod recoil;
pub mod sway;
pub mod weapon_data;

#[cfg(test)]
mod tests;
