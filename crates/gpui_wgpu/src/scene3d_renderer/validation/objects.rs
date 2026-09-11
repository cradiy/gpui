use anyhow::{Result, ensure};
use gpui::MeshDraw3d;

pub(super) fn validate_object_settings(object: &MeshDraw3d) -> Result<()> {
    ensure!(
        object
            .render_bounds
            .is_none_or(|bounds| bounds.iter().flatten().all(|v| v.is_finite())
                && (0..3).all(|axis| bounds[0][axis] <= bounds[1][axis])),
        "invalid object render bounds"
    );
    for set in object.texture_uv_sets() {
        ensure!(
            object.mesh.uv_at(set, 0).is_some(),
            "3D object {}: missing UV set {set}",
            object.output_id
        );
    }
    ensure!(
        !matches!(object.texture, gpui::MeshTexture3d::Image(_)) || object.sampling.is_valid(),
        "3D object {} has invalid image sampling",
        object.output_id
    );
    for map in object.lighting_textures().into_iter().flatten() {
        ensure!(
            map.sampling.is_valid(),
            "3D object {} has invalid material-map sampling",
            object.output_id
        );
    }
    ensure!(
        object.sort_depth.is_finite(),
        "3D object {} has invalid sort depth",
        object.output_id
    );
    ensure!(
        object.occlusion_strength.is_finite() && (0. ..=1.).contains(&object.occlusion_strength),
        "object {} has invalid occlusion strength",
        object.output_id
    );
    ensure!(
        object.normal_scale.is_finite() && object.normal_scale >= 0.,
        "3D object {} has invalid normal scale",
        object.output_id
    );
    ensure!(
        object.lighting_textures()[2]
            .is_none_or(|map| object.mesh.tangent_uv_set() == Some(map.uv_set)),
        "3D object {}: normal maps require mesh tangents for the selected UV set",
        object.output_id
    );
    ensure!(
        object.pbr.is_none_or(|pbr| pbr.is_valid()),
        "3D object {} has invalid PBR parameters",
        object.output_id
    );
    ensure!(
        object
            .model
            .iter()
            .flatten()
            .chain(object.normal.iter().flatten())
            .chain([
                &object.color.r,
                &object.color.g,
                &object.color.b,
                &object.color.a,
                &object.alpha_cutoff
            ])
            .all(|value| value.is_finite()),
        "3D object {} contains non-finite parameters",
        object.output_id
    );
    ensure!(
        object.alpha_cutoff >= 0.,
        "3D object {} has a negative alpha cutoff",
        object.output_id
    );
    Ok(())
}
