use gpui::{
    Bounds, EffectShader, EffectUniforms, Hsla, PaintEffect, Pixels, Point, Result, Rgba, Size,
    Window,
};

const ANCHOR_SLOT: usize = 0;
const TARGET_SLOT: usize = 1;
const SHAPE_SLOT: usize = 2;
const FILL_SLOT: usize = 3;

/// One rounded region in a sticky-shape field.
#[derive(Clone, Copy, Debug, Default)]
pub struct StickyShape {
    pub center: Point<Pixels>,
    pub size: Size<Pixels>,
    pub radius: Pixels,
}

impl StickyShape {
    pub fn new(center: Point<Pixels>, size: Size<Pixels>, radius: Pixels) -> Self {
        Self {
            center,
            size,
            radius,
        }
    }
}

/// Returns the GPU shader used by [`paint_sticky_shapes`].
pub fn sticky_shape_shader() -> EffectShader {
    EffectShader::wgsl(include_str!("shaders/sticky_shape.wgsl"))
}

/// Paints two rounded shapes and an elastic, pinched bridge between them.
///
/// Shape coordinates are local to `bounds`. `tension` normally follows the
/// drag distance from `0.0` (relaxed) to `1.0` (almost detached). The entire
/// field is evaluated in one GPU effect, avoiding seams where the bridge joins
/// either shape.
pub fn paint_sticky_shapes(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    anchor: StickyShape,
    target: StickyShape,
    tension: f32,
    fill: Hsla,
) -> Result<()> {
    let scale = window.scale_factor();
    let fill: Rgba = fill.into();
    let uniforms = EffectUniforms::new()
        .with_slot(ANCHOR_SLOT, shape_slot(anchor, scale))
        .with_slot(TARGET_SLOT, shape_slot(target, scale))
        .with_slot(
            SHAPE_SLOT,
            [
                anchor.radius.as_f32().max(0.0) * scale,
                target.radius.as_f32().max(0.0) * scale,
                tension.clamp(0.0, 1.0),
                scale,
            ],
        )
        .with_slot(FILL_SLOT, [fill.r, fill.g, fill.b, fill.a]);

    window.paint_effect(
        PaintEffect::new(bounds, sticky_shape_shader())
            .uniforms(uniforms)
            .corner_radii(Default::default()),
    )
}

fn shape_slot(shape: StickyShape, scale: f32) -> [f32; 4] {
    [
        shape.center.x.as_f32() * scale,
        shape.center.y.as_f32() * scale,
        shape.size.width.as_f32().max(0.0) * 0.5 * scale,
        shape.size.height.as_f32().max(0.0) * 0.5 * scale,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticky_shape_shader_is_valid_wgsl() {
        let source = gpui::compose_effect_shader_wgsl(&sticky_shape_shader());
        let module = naga::front::wgsl::parse_str(&source).expect("sticky shape shader must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("sticky shape shader must validate");
    }
}
