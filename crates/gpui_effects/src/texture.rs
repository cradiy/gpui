use gpui::EffectShader;

/// Two-image depth fog: linear color first, camera-forward R32Float depth second.
/// Slot 0.xy contains the start/end distances; slot 1 is linear fog RGBA, with
/// alpha controlling fog strength. Zero depth leaves background unchanged.
/// Color alpha is preserved. Sample depth with nearest filtering.
pub fn depth_fog_shader() -> EffectShader {
    EffectShader::wgsl_two_images(include_str!("shaders/depth_fog.wgsl"))
}

/// Maps linear HDR color to display-encoded sRGB while preserving alpha.
/// Slot 0.x is exposure in stops; slot 0.y greater than 0.5 selects Reinhard
/// mapping, otherwise exposed radiance is clipped to the display range.
/// Output storage must not apply a second sRGB encoding.
pub fn hdr_tone_map_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/hdr_tone_map.wgsl"))
}
