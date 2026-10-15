//! Third-person character: see the module docs further down (filled in by later steps).
use crate::viewmodel::{ArmsRoot, GunRoot, root_of_entity};
use crate::weapon::Inventory;
use bevy::prelude::*;

pub struct CharacterPlugin;
impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        info!("build {}", env!("CARGO_MANIFEST_DIR"));
        if std::env::var("BODYCAM_GRIPDUMP").is_ok() {
            app.add_systems(Update, grip_dump);
        }
    }
}

/// Dev: BODYCAM_GRIPDUMP=1 equips every gun in turn and prints where the FP arms' hands sit in the gun's `root` joint frame
/// (hip idle) -> assets/grips.json (the third-person hands hold the gun there).
fn grip_dump(
    time: Res<Time>,
    mut inv: Single<&mut Inventory>,
    guns: Query<(Entity, &GunRoot)>,
    arms: Query<Entity, With<ArmsRoot>>,
    names: Query<(Entity, &Name, &GlobalTransform)>,
    parents: Query<&ChildOf>,
    mut st: Local<(usize, f32, Vec<String>)>,
) {
    let now = time.elapsed_secs();
    if (now * 2.0) as u32 % 10 == 0 { info!("gripdump t={now:.1} idx={} next={:.1}", st.0, st.1); }
    if st.2.is_empty() {
        st.2 = inv.slots.iter().take(2).chain(inv.pool.iter()).filter(|w| w.kind == crate::weapon::ItemKind::Gun).map(|w| w.name.clone()).collect();
        st.1 = now + 8.0;
    }
    if now < st.1 || st.0 >= st.2.len() {
        return;
    }
    let row = st.2[st.0].clone();
    let model = inv.slots.iter().chain(inv.pool.iter()).find(|w| w.name == row).and_then(|w| crate::viewmodel::gun_model(&w.name, &w.anims));
    if inv.slots[inv.current].name != row {
        let other = if row == "Glock17" { "M4A1" } else { "Glock17" };
        inv.equip(&row, other);
        st.1 = now + 8.0;
        return;
    }
    st.0 += 1;
    st.1 = now + 1.5;
    let (Some(model), Some((gun, _))) = (model, guns.iter().find(|(_, g)| Some(g.0) == model)) else { return };
    let (Ok(arms), Some(root)) = (arms.single(), names.iter().find(|(e, n, _)| n.as_str().eq_ignore_ascii_case("root") && root_of_entity(*e, gun, &parents))) else { return };
    let inv_root = root.2.affine().inverse();
    let mut out = format!("GRIP \"{model}\": {{");
    for side in ["r", "l"] {
        let Some((_, _, g)) = names.iter().find(|(e, n, _)| n.as_str() == format!("hand_{side}") && root_of_entity(*e, arms, &parents)) else { return };
        let (_, r, t) = (inv_root * g.affine()).to_scale_rotation_translation();
        out += &format!("\"{side}\": [{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}], ", t.x, t.y, t.z, r.x, r.y, r.z, r.w);
    }
    info!("{}}},", out.trim_end_matches(", "));
}
