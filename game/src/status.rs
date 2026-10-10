//! WW_SurvivorStatus (health / stamina / stress) and AC_BodycamCamera's out-of-breath state, ported from the
//! decompiled Blueprints (extracted/bp/WW_SurvivorStatus.txt, AC_BodycamCamera.txt, GA_Character_Sprint.txt).
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
    /// AC_BodycamCamera.OutOfBreath (0..1), its target and the reset timer.
    pub out_of_breath: f32,
    target_out_of_breath: f32,
    breath_reset_timer: Option<f32>,
    tick: f32,
    was_sprinting: bool,
}

impl Default for SurvivorStatus {
    fn default() -> Self {
        // Class defaults: Health 100, HealthMax 100, Stress 0.1, Stamina 0.
        SurvivorStatus {
            health: 100.0,
            health_max: 100.0,
            stamina: 0.0,
            stress: 0.1,
            out_of_breath: 0.0,
            target_out_of_breath: 0.0,
            breath_reset_timer: None,
            tick: 0.0,
            was_sprinting: false,
        }
    }
}

const TIME_TO_RESET_BREATH: f32 = 3.0; // AC_BodycamCamera.TimeToResetBreath

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

impl SurvivorStatus {
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

pub struct StatusPlugin;
impl Plugin for StatusPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SurvivorStatus>().add_message::<PlayerDamage>().add_message::<BulletFly>().add_systems(Update, (update, stress_sources));
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
    let dt = time.delta_secs();
    let speed_cm = Vec2::new(p.vel.x, p.vel.z).length() * 100.0;
    // "Update Survivor Stat": looping 1 s timer started on BeginPlay.
    s.tick += dt;
    while s.tick >= 1.0 {
        s.tick -= 1.0;
        if s.health < s.health_max {
            s.health += 2.0;
        }
        if p.grounded && speed_cm > 20.0 && p.sprinting {
            s.stamina = (s.stamina + 0.1).clamp(0.0, 1.0);
        } else {
            s.stress = (s.stress - 0.1).clamp(0.1, 1.0);
            s.stamina = (s.stamina - 0.05).clamp(0.0, 1.0);
        }
    }
    // GA_Character_Sprint OnEndAbility -> AC_BodycamCamera.StartOutOfBreath.
    if s.was_sprinting && !p.sprinting {
        s.target_out_of_breath = 1.0;
    }
    s.was_sprinting = p.sprinting;
    // UpdateOutOfBreath: FInterpTo 5; once near the target (> 0.2) arm a 3 s timer that calls StopOutOfBreath.
    s.out_of_breath = finterp_to(s.out_of_breath, s.target_out_of_breath, dt, 5.0);
    if let Some(t) = &mut s.breath_reset_timer {
        *t -= dt;
        if *t <= 0.0 {
            s.breath_reset_timer = None;
            s.target_out_of_breath = 0.0;
        }
    } else if (s.out_of_breath - s.target_out_of_breath).abs() <= 0.5 && s.out_of_breath > 0.2 {
        s.breath_reset_timer = Some(TIME_TO_RESET_BREATH);
    }
}

#[cfg(test)]
mod tests {
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
