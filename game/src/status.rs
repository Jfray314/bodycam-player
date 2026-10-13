//! WW_SurvivorStatus (health / stamina / stress), ported from the decompiled Blueprints and fitted to the bot-fight
//! recordings (extracted/bp/WW_SurvivorStatus.txt; tools/probe_status.py -> assets/status_reference.json).
//! In Bodycam "Stamina" is really fatigue: it rises while sprinting and only drives the breathing sound
//! (canAccelerate is never set false, so it never blocks sprinting).
use crate::player::Player;
use bevy::prelude::*;

#[derive(Resource)]
pub struct SurvivorStatus {
    pub health: f32,
    pub health_max: f32,
    pub stamina: f32,
    pub stress: f32,
    /// Seconds into the 1 s "Update Survivor Stat" timer.
    pub tick: f32,
}

impl Default for SurvivorStatus {
    fn default() -> Self {
        // Class defaults: Health 100, HealthMax 100, Stress 0.1, Stamina 0.
        SurvivorStatus { health: 100.0, health_max: 100.0, stamina: 0.0, stress: 0.1, tick: 0.0 }
    }
}

/// UE FMath::FInterpTo.
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

/// UE MapRangeClamped.
pub fn map_range_clamped(v: f32, a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    if a0 == a1 {
        return b1;
    }
    (b0 + (v - a0) / (a1 - a0) * (b1 - b0)).clamp(b0.min(b1), b0.max(b1))
}

/// GA_Fire_Base "ReceiveFireEvent": IncreaseStress(MapRangeClamped(FireRate, 0.05..0.8 -> 0.025..0.35)).
pub fn fire_stress(fire_rate: f32) -> f32 {
    map_range_clamped(fire_rate, 0.05, 0.8, 0.025, 0.35)
}

/// AC_BodycamAnimationComponent "Stress Tick" subtle alpha: FInterpTo(map(speed 0..100 cm/s -> 0.1..1), speed 10); 0 target while aiming.
/// Fitted to the bot-fight recordings (tools/probe_status.py).
pub fn stress_subtle_step(subtle: f32, dt: f32, speed_cm: f32, aiming: bool) -> f32 {
    finterp_to(subtle, if aiming { 0.0 } else { map_range_clamped(speed_cm, 0.0, 100.0, 0.1, 1.0) }, dt, 10.0)
}

/// Strong alpha: aiming ? map(stress 0.5..1 -> 0..1) : map(stress 0..1 -> 0..4) (no interpolation).
pub fn stress_strong(stress: f32, aiming: bool) -> f32 {
    if aiming { map_range_clamped(stress, 0.5, 1.0, 0.0, 1.0) } else { map_range_clamped(stress, 0.0, 1.0, 0.0, 4.0) }
}

impl SurvivorStatus {
    /// "Update Survivor Stat" looping 1 s timer: +2 health, then sprinting on ground (Gait 1, speed > 20) -> stamina +0.1,
    /// otherwise stress -0.1 (clamped 0.1..1) and stamina -0.05. IncreaseStress itself is unclamped.
    pub fn step(&mut self, dt: f32, sprinting_on_ground: bool) {
        self.tick += dt;
        while self.tick >= 1.0 {
            self.tick -= 1.0;
            if self.health < self.health_max {
                self.health += 2.0;
            }
            if sprinting_on_ground {
                self.stamina = (self.stamina + 0.1).clamp(0.0, 1.0);
            } else {
                self.stress = (self.stress - 0.1).clamp(0.1, 1.0);
                self.stamina = (self.stamina - 0.05).clamp(0.0, 1.0);
            }
        }
    }

    /// WW_SurvivorStatus.IncreaseStress: unclamped (only the 1 s decay clamps to 0.1..1).
    pub fn increase_stress(&mut self, d: f32) {
        self.stress += d;
    }

    /// Bodycam_Player.IsStress: RandomBoolWithWeight(clamp(Stamina + Stress + MapRange(Health 0..100 -> 1..0), 0, 1)).
    pub fn is_stress(&self) -> bool {
        let w = (self.stamina + self.stress + map_range_clamped(self.health, 0.0, 100.0, 1.0, 0.0)).clamp(0.0, 1.0);
        rand::random::<f32>() < w
    }
}

/// Damage to the player (ApplyPointDamage -> K2_OnHealthChanged); `flinch` ponytail: Bodycam_Player's 0.1-0.15 s flinch not ported.
#[derive(Message)]
pub struct PlayerDamage(pub f32);

/// Bodycam_Player.BulletFly: a bullet passed within `distance` cm of the player (whiz): Stress += 0.3.
#[derive(Message)]
pub struct BulletFly {
    pub distance_cm: f32,
}

/// GA_Fire_Base.CheckNearlyPlayer (player within MaxWhizzByRadius 400 cm of the bullet segment) -> AC_BodycamCamera.GettingShot.
#[derive(Message)]
pub struct NearMiss {
    pub row: String,
    pub origin_dist_cm: f32,
    pub closest_dist_cm: f32,
}

pub struct StatusPlugin;
impl Plugin for StatusPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SurvivorStatus>().add_message::<PlayerDamage>().add_message::<BulletFly>().add_message::<NearMiss>().add_systems(Update, (update, stress_sources));
    }
}

fn stress_sources(
    mut s: ResMut<SurvivorStatus>,
    mut fired: MessageReader<crate::weapon::Fired>,
    mut whiz: MessageReader<BulletFly>,
    mut dmg: MessageReader<PlayerDamage>,
    mut dead_for: Local<f32>,
    time: Res<Time>,
) {
    for f in fired.read() {
        s.increase_stress(fire_stress(f.fire_rate));
    }
    for _ in whiz.read() {
        s.increase_stress(0.3);
    }
    for d in dmg.read() {
        s.health -= d.0;
    }
    // Test-bench death: no ragdoll/respawn flow ported, so just refill after 3 s.
    if s.health <= 0.0 {
        *dead_for += time.delta_secs();
        if *dead_for > 3.0 {
            *dead_for = 0.0;
            s.health = s.health_max;
            s.stress = 0.1;
        }
    }
}

fn update(time: Res<Time>, p: Single<&Player>, mut s: ResMut<SurvivorStatus>) {
    let speed_cm = Vec2::new(p.vel.x, p.vel.z).length() * 100.0;
    s.step(time.delta_secs(), p.grounded && speed_cm > 20.0 && p.sprinting);
    // ponytail: AC_BodycamCamera.OutOfBreath is never advanced in the real game (UpdateOutOfBreath has no caller;
    // OutOfBreath/Tremor/HandFatigue stay 0 in every recording), so nothing is ported.
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Replays the recorded inputs of the bot-fight recordings (assets/status_reference.json, from tools/probe_status.py)
    /// through the status rules and compares against the recorded curves.
    #[test]
    fn replay_matches_bot_fight_recordings() {
        let j: Value = serde_json::from_str(include_str!("../../assets/status_reference.json")).unwrap();
        let col = |r: &Value, k: &str| r[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect::<Vec<_>>();
        for (name, r) in j["recordings"].as_object().unwrap() {
            let (t, sp, aim) = (col(r, "t"), col(r, "speed_cm"), col(r, "aiming"));
            let (shots, fr, dmg) = (col(r, "shots"), col(r, "fire_rate"), col(r, "damage"));
            let exp: Vec<Vec<f32>> = ["stress", "health", "stamina", "subtle", "strong"].iter().map(|k| col(r, k)).collect();
            let mut s = SurvivorStatus { health: exp[1][0], stress: exp[0][0], stamina: exp[2][0], tick: 1.0 - (r["first_tick"].as_f64().unwrap() as f32 - t[0]), ..default() };
            let mut sub = exp[3][0];
            let (mut se, mut worst) = ([0f32; 5], [0f32; 5]);
            for i in 1..t.len() {
                let dt = t[i] - t[i - 1];
                s.step(dt, sp[i] > 150.0); // ponytail: sprint gait proxied by speed (walk 100/125, sprint 325)
                if shots[i] > 0.0 {
                    s.increase_stress(shots[i] * fire_stress(fr[i]));
                }
                s.health -= dmg[i];
                sub = stress_subtle_step(sub, dt, sp[i], aim[i] > 0.5);
                let got = [s.stress, s.health, s.stamina, sub, stress_strong(s.stress, aim[i] > 0.5)];
                for k in 0..5 {
                    let e = got[k] - exp[k][i];
                    se[k] += e * e;
                    worst[k] = worst[k].max(e.abs());
                }
            }
            let n = t.len() as f32;
            let rms: Vec<f32> = se.iter().map(|x| (x / n).sqrt()).collect();
            println!("{name}: rms stress/health/stamina/subtle/strong = {rms:.4?} max = {worst:.3?}");
            // tolerances: one 1 s tick landing a sample off costs 0.1 stress / 2 hp / 0.05-0.1 stamina for a frame or two
            assert!(rms[0] < 0.03 && rms[1] < 1.0 && rms[2] < 0.03 && rms[3] < 0.05 && rms[4] < 0.12, "{name}: {rms:?}");
        }
    }

    #[test]
    fn finterp_matches_ue() {
        assert_eq!(super::finterp_to(0.0, 1.0, 0.1, 5.0), 0.5);
        assert_eq!(super::finterp_to(0.0, 1.0, 1.0, 5.0), 1.0);
        assert_eq!(super::finterp_to(0.3, 1.0, 0.1, 0.0), 1.0);
    }

    #[test]
    fn fire_stress_matches_maprange() {
        assert_eq!(super::fire_stress(0.0), 0.025);
        assert_eq!(super::fire_stress(2.0), 0.35);
        assert!((super::fire_stress(0.425) - 0.1875).abs() < 1e-6);
    }
}
