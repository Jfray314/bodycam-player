// Soft particles: StandardMaterial forward shading, alpha faded by the distance to the opaque depth behind the sprite
// (UE DepthFade / Cascade soft particle). Needs DepthPrepass on the camera. Reverse-Z infinite far: view distance = near / depth.
import bevy_pbr::{
    render::{
        pbr_fragment::pbr_input_from_standard_material,
        pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
        forward_io::{VertexOutput, FragmentOutput},
        view_transformations::depth_ndc_to_view_z,
    },
    prepass::utils::prepass_depth,
};

@group(3) @binding(100) var<uniform> soft_dist: f32;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material.flags, pbr_input.material.alpha_cutoff, pbr_input.material.base_color);
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    let scene = prepass_depth(in.position, 0u);
    let d = depth_ndc_to_view_z(in.position.z) - depth_ndc_to_view_z(scene); // view z is negative: surface behind = more negative
    out.color.a *= clamp(d / soft_dist, 0.0, 1.0);
    return out;
}
