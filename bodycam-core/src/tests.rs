//! Golden numbers were printed by the game's own `bodycam_camera` / `look` / `CtlRecoil` (game/src/player.rs) driven with
//! the same input sequence (fixed fire randoms), converted to UE convention. If player.rs maths changes, regenerate.
use super::*;

/// frame, pos (x,y,z), rot (pitch,yaw,roll), ctl (yaw,pitch)
const GOLD: &[(usize, [f32; 3], [f32; 3], [f32; 2])] = &[
    (59, [-0.05473, 1.35330, -0.35251], [0.0232, -0.8645, -1.4369], [18.0, 0.0]),
    (119, [-0.04679, 1.52653, -0.34318], [-0.0959, 0.3394, 0.6933], [18.0, -6.0]),
    (179, [-0.09333, 1.47996, -0.36193], [0.1203, 0.1200, 4.2647], [18.0, -6.0]),
    (215, [-0.02832, 1.46629, -0.36741], [-0.1735, -0.2654, 1.7331], [18.0, -6.0]),
    (225, [-0.04203, 1.34750, -0.37163], [-4.5441, -0.1916, 1.1646], [18.0, -6.0]),
    (241, [-0.04754, 1.39747, -0.21540], [-2.4709, 0.0250, 1.3804], [17.7569, -5.3924]),
    (250, [-0.04931, 1.45551, -0.22143], [-0.8317, -0.3129, 2.7591], [17.2917, -4.2292]),
    (259, [-0.04959, 1.51151, -0.22836], [0.8473, -0.4127, 0.1900], [17.5781, -4.9453]),
    (280, [-0.04106, 1.32786, -0.24511], [-0.0124, -0.0554, -0.0087], [17.9990, -5.9975]),
    (330, [-0.28077, 1.41514, -0.23886], [-0.0157, -0.0492, -14.4855], [18.0, -6.0]),
    (359, [-0.08145, 1.62494, -0.23928], [-0.0238, -0.0011, -15.4416], [18.0, -6.0]),
];

#[test]
fn matches_game_sequence() {
    let mut core = Core::new(1);
    let dt = 1.0f32 / 60.0;
    for n in 0..360usize {
        let r = |a: usize, b: usize| (a..b).contains(&n);
        let mut flags = 0;
        for (cond, f) in [(!r(200, 215), GROUNDED), (r(270, 330), CROUCHED), (r(120, 240), SPRINTING), (r(240, 258), TRIGGER_HELD), (r(300, 340), LEAN_PRESSED), (n == 215, LANDED)] {
            if cond {
                flags |= f;
            }
        }
        if [240, 245, 250].contains(&n) {
            core.fire_with(&ControllerRecoil { horizontal: (-0.1, -0.1), vertical: (0.25, 0.25), smoothing: 50.0, damping: 15.0, compensation: 1.0 }, [0.5; 5]);
        }
        let out = core.step(&Input {
            dt,
            look_yaw_deg: if r(30, 60) { 0.6 } else { 0.0 },
            look_pitch_deg: if r(60, 90) { -0.2 } else { 0.0 },
            move_x: if r(120, 180) { 1.0 } else { 0.0 },
            speed: if n < 120 { 1.25 } else if n < 240 { 3.25 } else { 0.0 },
            lean_target: if r(300, 340) { -1.0 } else { 0.0 },
            flags,
        });
        if let Some((_, pos, rot, ctl)) = GOLD.iter().find(|g| g.0 == n) {
            let close = |a: &[f32], b: &[f32], tol: f32| a.iter().zip(b).all(|(x, y)| (x - y).abs() < tol);
            assert!(close(&out.pos, pos, 1e-3), "frame {n} pos {:?} vs {:?}", out.pos, pos);
            assert!(close(&out.rot, rot, 5e-3), "frame {n} rot {:?} vs {:?}", out.rot, rot);
            assert!(close(&[out.ctl_yaw_deg, out.ctl_pitch_deg], ctl, 5e-3), "frame {n} ctl");
        }
    }
}

#[test]
fn c_abi_roundtrip() {
    unsafe {
        let c = bc_create(7);
        assert!(!c.is_null());
        let inp = BcInput { dt: 1.0 / 60.0, flags: GROUNDED, ..Default::default() };
        let mut out = BcOutput::default();
        for _ in 0..10 {
            bc_step(c, &inp, &mut out);
        }
        assert!((out.pos[1] - (0.9 + 0.71852)).abs() < 0.02); // standing idle: capsule centre + measured chest mount
        bc_destroy(c);
    }
}
