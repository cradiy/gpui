use gpui::{
    App, BackdropShader, Bounds, Corners, Div, EffectUniforms, ElementId, Hsla, InteractiveElement,
    Interactivity, IntoElement, PaintBackdropEffect, ParentElement, Pixels, Point, RenderOnce,
    Rgba, StatefulInteractiveElement, StyleRefinement, Styled, Window, div, hsla, point, px,
};

const OPTICS_SLOT: usize = 0;
const TINT_SLOT: usize = 1;
const SURFACE_SLOT: usize = 2;
const LIGHT_SLOT: usize = 3;
const CORNERS_SLOT: usize = 4;

/// Optical parameters for [`LiquidGlass`]. Layout and foreground styling use [`Styled`].
#[derive(Clone, Copy, Debug, PartialEq, uic_macros::Chainable)]
pub struct LiquidGlassAppearance {
    /// Blur radius of the background, in logical pixels.
    pub blur_radius: Pixels,
    /// Sharp background contribution in `0..=1`; `1` is clear glass.
    pub clarity: f32,
    /// Maximum inward background displacement, in logical pixels.
    pub refraction: Pixels,
    /// Width of the curved edge region, in logical pixels.
    pub thickness: Pixels,
    /// Relative RGB separation at refracted edges in `0..=0.1`; zero disables it.
    pub dispersion: f32,
    /// Background color wash; alpha controls its contribution.
    pub tint: Hsla,
    /// Nonnegative saturation multiplier for the sampled background.
    pub saturation: f32,
    /// Nonnegative brightness multiplier for the sampled background.
    pub brightness: f32,
    /// Directional reflection strength in `0..=1`.
    pub highlight: f32,
    /// Inner shading opposite the light, in `0..=1`; not an exterior drop shadow.
    pub edge_shadow: f32,
    /// Width of the fine reflective rim, in logical pixels.
    pub rim_width: Pixels,
    /// Direction toward the light; negative x/y points toward the top-left.
    pub light_direction: Point<f32>,
}

impl LiquidGlassAppearance {
    /// A softened, lightly tinted surface that balances detail and readability.
    pub fn regular() -> Self {
        Self {
            blur_radius: px(8.0),
            clarity: 0.22,
            refraction: px(7.0),
            thickness: px(16.0),
            dispersion: 0.015,
            tint: hsla(0.0, 0.0, 1.0, 0.12),
            saturation: 1.04,
            brightness: 1.0,
            highlight: 0.34,
            edge_shadow: 0.12,
            rim_width: px(0.8),
            light_direction: point(-0.6, -0.8),
        }
    }

    /// More transparent glass for compact controls over a legible background.
    pub fn clear() -> Self {
        Self {
            blur_radius: px(3.0),
            clarity: 0.78,
            tint: hsla(0.0, 0.0, 1.0, 0.045),
            thickness: px(12.0),
            refraction: px(6.0),
            ..Self::regular()
        }
    }

    /// A dark neutral wash for light foreground content.
    pub fn dark() -> Self {
        Self {
            tint: hsla(0.61, 0.12, 0.10, 0.28),
            brightness: 0.96,
            ..Self::regular()
        }
    }

    fn uniforms(self, corners: Corners<Pixels>, scale: f32) -> EffectUniforms {
        let tint: Rgba = self.tint.into();
        EffectUniforms::new()
            .with_slot(
                OPTICS_SLOT,
                [
                    self.saturation.max(0.0),
                    self.brightness.max(0.0),
                    self.refraction.as_f32().max(0.0) * scale,
                    self.thickness.as_f32().max(0.0) * scale,
                ],
            )
            .with_slot(TINT_SLOT, [tint.r, tint.g, tint.b, tint.a])
            .with_slot(
                SURFACE_SLOT,
                [
                    self.highlight.clamp(0.0, 1.0),
                    self.edge_shadow.clamp(0.0, 1.0),
                    self.dispersion.clamp(0.0, 0.1),
                    self.clarity.clamp(0.0, 1.0),
                ],
            )
            .with_slot(
                LIGHT_SLOT,
                [
                    self.light_direction.x,
                    self.light_direction.y,
                    self.rim_width.as_f32().max(0.0) * scale,
                    0.0,
                ],
            )
            .with_slot(
                CORNERS_SLOT,
                [
                    corners.top_left.as_f32() * scale,
                    corners.top_right.as_f32() * scale,
                    corners.bottom_right.as_f32() * scale,
                    corners.bottom_left.as_f32() * scale,
                ],
            )
    }
}

impl Default for LiquidGlassAppearance {
    fn default() -> Self {
        Self::regular()
    }
}

/// A layout-neutral surface that applies a refractive material to its background.
///
/// The material samples content already painted in the same window. Children
/// are painted afterward and remain sharp. Sizing, individual corner radii,
/// foreground colors, padding, shadows, and interaction use the normal Div APIs.
pub struct LiquidGlass {
    div: Div,
    appearance: LiquidGlassAppearance,
}

impl LiquidGlass {
    pub fn new() -> Self {
        Self::with_appearance(LiquidGlassAppearance::default())
    }

    pub fn with_appearance(appearance: LiquidGlassAppearance) -> Self {
        Self {
            div: div(),
            appearance,
        }
    }

    pub fn appearance(mut self, appearance: LiquidGlassAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Assigns a stable id for hover, click, focus, and other Div interactions.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.div.interactivity().element_id = Some(id.into());
        self
    }
}

impl Default for LiquidGlass {
    fn default() -> Self {
        Self::new()
    }
}

impl Styled for LiquidGlass {
    fn style(&mut self) -> &mut StyleRefinement {
        self.div.style()
    }
}

impl InteractiveElement for LiquidGlass {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.div.interactivity()
    }
}

impl StatefulInteractiveElement for LiquidGlass {}

impl ParentElement for LiquidGlass {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.div.extend(elements);
    }
}

impl IntoElement for LiquidGlass {
    type Element = gpui::ViewElement<Self>;

    fn into_element(self) -> Self::Element {
        gpui::ViewElement::new(self)
    }
}

impl RenderOnce for LiquidGlass {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let appearance = self.appearance;
        self.div.on_paint_before_children(
            move |bounds: Bounds<Pixels>, style, window: &mut Window, _| {
                let corners = style.corner_radii.to_pixels(window.rem_size());
                paint_liquid_glass(bounds, corners, appearance, window);
            },
        )
    }
}

/// Paints the material without owning layout, children, or interaction.
///
/// Bounds, corner radii, and appearance lengths are in logical pixels. Call
/// before painting foreground content, for example in a Div's
/// `on_paint_before_children` hook. Only previously painted content in the same
/// window is sampled.
pub fn paint_liquid_glass(
    bounds: Bounds<Pixels>,
    corners: Corners<Pixels>,
    appearance: LiquidGlassAppearance,
    window: &mut Window,
) {
    if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
        return;
    }
    let corners = corners.clamp_radii_for_quad_size(bounds.size);
    window.paint_backdrop_effect(
        PaintBackdropEffect::new(
            bounds,
            appearance.blur_radius.max(px(0.0)),
            liquid_glass_shader(),
        )
        .uniforms(appearance.uniforms(corners, window.scale_factor()))
        .corner_radii(corners),
    );
}

/// Portable backdrop shader used by [`LiquidGlass`].
pub fn liquid_glass_shader() -> BackdropShader {
    BackdropShader::wgsl(include_str!("shaders/liquid_glass.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_parameters_scale_geometry_but_not_light_or_color() {
        let appearance = LiquidGlassAppearance::clear()
            .refraction(px(5.0))
            .thickness(px(12.0))
            .rim_width(px(0.5))
            .dispersion(0.03)
            .light_direction(point(0.4, -0.8));
        let corners = Corners {
            top_left: px(1.0),
            top_right: px(2.0),
            bottom_right: px(3.0),
            bottom_left: px(4.0),
        };
        let uniforms = appearance.uniforms(corners, 2.0);
        assert_eq!(uniforms.slots()[OPTICS_SLOT], [1.04, 1.0, 10.0, 24.0]);
        assert_eq!(uniforms.slots()[LIGHT_SLOT], [0.4, -0.8, 1.0, 0.0]);
        assert_eq!(uniforms.slots()[CORNERS_SLOT], [2.0, 4.0, 6.0, 8.0]);
        assert_eq!(uniforms.slots()[SURFACE_SLOT], [0.34, 0.12, 0.03, 0.78]);
    }

    #[test]
    fn optical_ranges_are_bounded() {
        let uniforms = LiquidGlassAppearance::default()
            .clarity(2.0)
            .dispersion(1.0)
            .highlight(-1.0)
            .edge_shadow(2.0)
            .refraction(px(-2.0))
            .uniforms(Corners::default(), 1.0);
        assert_eq!(uniforms.slots()[SURFACE_SLOT], [0.0, 1.0, 0.1, 1.0]);
        assert_eq!(uniforms.slots()[OPTICS_SLOT][2], 0.0);
    }

    #[test]
    fn container_supports_styles_children_and_interaction() {
        let glass = LiquidGlass::new()
            .id("glass-button")
            .w(px(160.0))
            .rounded_full()
            .p_2()
            .text_color(gpui::rgb(0xffffff))
            .cursor_pointer()
            .on_click(|_, _, _| {})
            .child("Play");
        let _: gpui::ViewElement<LiquidGlass> = glass.into_element();
    }

    #[test]
    fn liquid_glass_shader_is_valid_wgsl() {
        let source = gpui::compose_backdrop_shader_wgsl(&liquid_glass_shader());
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("liquid glass shader must validate");
        let (_, translation) = naga::back::msl::write_string(
            &module,
            &info,
            &naga::back::msl::Options {
                lang_version: (2, 0),
                ..Default::default()
            },
            &naga::back::msl::PipelineOptions::default(),
        )
        .expect("liquid glass shader must translate to MSL");
        assert!(translation.entry_point_names.iter().all(Result::is_ok));
    }
}
