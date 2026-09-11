use anyhow::{Result, ensure};
use gpui::Scene3dFrame;

mod objects;

pub(crate) fn validate_frame_settings(
    frame: &Scene3dFrame,
    max_texture_dimension: u32,
    shaded: bool,
) -> Result<()> {
    ensure!(
        frame.shadow_is_valid(),
        "invalid directional shadow parameters or source"
    );
    ensure!(
        frame
            .directional_shadow
            .is_none_or(|shadow| shadow.resolution <= max_texture_dimension),
        "shadow resolution exceeds device limits"
    );
    if let Some(lights) = &frame.lights {
        ensure!(
            lights.len() <= gpui::MAX_PUNCTUAL_LIGHTS_3D,
            "too many direct lights"
        );
        for (index, light) in lights.iter().enumerate() {
            ensure!(
                light.is_valid(),
                "direct light {index} has invalid parameters"
            );
        }
    }
    ensure!(
        frame
            .diffuse_environment
            .is_none_or(|environment| environment.is_valid()),
        "invalid diffuse environment parameters"
    );
    if shaded && let Some(environment) = &frame.specular_environment {
        ensure!(
            environment.is_valid(),
            "invalid specular environment parameters"
        );
        ensure!(
            environment.map.size() <= max_texture_dimension,
            "specular environment exceeds device texture dimensions"
        );
    }
    if shaded && let Some(background) = &frame.background {
        ensure!(
            background.is_valid(),
            "invalid environment background parameters"
        );
        ensure!(
            background
                .map
                .size()
                .iter()
                .all(|v| *v <= max_texture_dimension),
            "environment map exceeds device texture dimensions"
        );
    }
    ensure!(
        frame.color_output.is_valid(),
        "3D exposure must be finite and between -16 and 16 stops"
    );
    ensure!(
        frame
            .view_projection
            .iter()
            .flatten()
            .chain(frame.world_to_view.iter().flatten())
            .chain(&frame.camera_position)
            .chain(frame.orthographic_view_direction.iter().flatten())
            .chain(&frame.light_direction)
            .chain(&frame.light)
            .chain([&frame.ambient])
            .all(|value| value.is_finite()),
        "3D frame contains non-finite camera or light parameters"
    );
    ensure!(
        frame
            .orthographic_view_direction
            .is_none_or(|direction| direction.iter().any(|v| *v != 0.)),
        "3D orthographic view direction must be nonzero"
    );
    for object in frame.objects.iter() {
        objects::validate_object_settings(object)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
