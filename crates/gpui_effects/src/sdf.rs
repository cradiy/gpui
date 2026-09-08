use gpui::{
    Bounds, Canvas, EffectShader, EffectUniforms, PaintEffect, Pixels, Point, Result, Rgba, Size,
    Window, canvas, point, px,
};

/// Maximum number of independently transformed shapes in one SDF field.
pub const MAX_SDF_SHAPES: usize = gpui::EFFECT_UNIFORM_SLOTS - 2;

/// Surface-local position, uniform scale and rotation of one shape.
#[derive(Clone, Copy, Debug)]
pub struct SdfTransform {
    pub center: Point<Pixels>,
    pub scale: f32,
    /// Clockwise rotation in radians.
    pub rotation: f32,
}
impl Default for SdfTransform {
    fn default() -> Self {
        Self {
            center: point(px(0.), px(0.)),
            scale: 1.,
            rotation: 0.,
        }
    }
}

#[derive(Clone, Debug)]
enum Geometry {
    Circle(f32),
    RoundedRect([f32; 2], f32),
    Capsule(f32, f32),
}
#[derive(Clone, Copy, Debug)]
enum Operation {
    Union,
    Intersect,
    Subtract,
}
#[derive(Clone, Debug)]
enum Node {
    Shape {
        geometry: Geometry,
        color: Rgba,
        transform: SdfTransform,
    },
    Combine {
        left: Box<Node>,
        right: Box<Node>,
        operation: Operation,
        smooth: bool,
    },
}

/// Geometry and Boolean composition of a distance field.
/// Construct the tree once, then animate its leaves through [`SdfScene::set_transform`].
#[derive(Clone, Debug)]
pub struct SdfShape(Node);
impl SdfShape {
    /// Creates a circle in surface-local logical pixels.
    pub fn circle(center: Point<Pixels>, radius: Pixels, color: Rgba) -> Self {
        Self::leaf(center, Geometry::Circle(nonnegative(radius.into())), color)
    }
    /// Creates a centered rounded rectangle. Radius is limited to the shorter half-size.
    pub fn rounded_rect(
        center: Point<Pixels>,
        size: Size<Pixels>,
        radius: Pixels,
        color: Rgba,
    ) -> Self {
        let half = [
            nonnegative(f32::from(size.width)) * 0.5,
            nonnegative(f32::from(size.height)) * 0.5,
        ];
        Self::leaf(
            center,
            Geometry::RoundedRect(half, nonnegative(radius.into()).min(half[0]).min(half[1])),
            color,
        )
    }
    /// Creates a horizontal capsule. `length` is the distance between its semicircle centers.
    pub fn capsule(center: Point<Pixels>, length: Pixels, radius: Pixels, color: Rgba) -> Self {
        Self::leaf(
            center,
            Geometry::Capsule(nonnegative(length.into()) * 0.5, nonnegative(radius.into())),
            color,
        )
    }
    fn leaf(center: Point<Pixels>, geometry: Geometry, color: Rgba) -> Self {
        Self(Node::Shape {
            geometry,
            color,
            transform: SdfTransform {
                center,
                ..Default::default()
            },
        })
    }
    fn combine(self, other: Self, operation: Operation, smooth: bool) -> Self {
        Self(Node::Combine {
            left: Box::new(self.0),
            right: Box::new(other.0),
            operation,
            smooth,
        })
    }
    /// Keeps both fields without rounding the join.
    pub fn union(self, other: Self) -> Self {
        self.combine(other, Operation::Union, false)
    }
    /// Blends both fields and their colors using the scene's smoothing radius.
    pub fn smooth_union(self, other: Self) -> Self {
        self.combine(other, Operation::Union, true)
    }
    /// Keeps only the overlapping region.
    pub fn intersect(self, other: Self) -> Self {
        self.combine(other, Operation::Intersect, false)
    }
    /// Keeps the overlap with a rounded boundary.
    pub fn smooth_intersect(self, other: Self) -> Self {
        self.combine(other, Operation::Intersect, true)
    }
    /// Cuts `other` out of this field, retaining this field's color.
    pub fn subtract(self, other: Self) -> Self {
        self.combine(other, Operation::Subtract, false)
    }
    /// Cuts `other` out with a rounded boundary, retaining this field's color.
    pub fn smooth_subtract(self, other: Self) -> Self {
        self.combine(other, Operation::Subtract, true)
    }
}

/// Runtime edge and lighting settings. Distances are in logical pixels.
#[derive(Clone, Copy, Debug)]
pub struct SdfOptions {
    /// Blend radius used by smooth Boolean operations. Zero gives hard operations.
    pub smoothing: Pixels,
    /// Centered stroke width. Zero disables the stroke.
    pub stroke_width: Pixels,
    /// Outer glow falloff distance. Zero disables outer glow.
    pub outer_glow: Pixels,
    pub outer_glow_opacity: f32,
    /// Inner edge light falloff distance. Zero disables inner light.
    pub inner_glow: Pixels,
    pub inner_glow_intensity: f32,
    /// Interior opacity. Zero leaves only the stroke and outer glow.
    pub fill_opacity: f32,
}
impl Default for SdfOptions {
    fn default() -> Self {
        Self {
            smoothing: px(32.),
            stroke_width: px(0.),
            outer_glow: px(0.),
            outer_glow_opacity: 0.35,
            inner_glow: px(0.),
            inner_glow_intensity: 0.3,
            fill_opacity: 1.,
        }
    }
}

/// A reusable SDF program with independently movable leaves.
/// Geometry, colors and tree topology are fixed; transforms and options update uniforms only.
#[derive(Clone)]
pub struct SdfScene {
    shader: EffectShader,
    transforms: Vec<SdfTransform>,
    options: SdfOptions,
}
impl SdfScene {
    /// Compiles a composition with at most [`MAX_SDF_SHAPES`] leaves.
    /// Leaf indices follow left-to-right construction order, including cutters.
    pub fn new(shape: SdfShape) -> Result<Self> {
        let mut source = include_str!("shaders/sdf.wgsl").to_owned();
        source.push_str("\nfn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {\nlet position = input.uv * input.size;\n");
        let mut transforms = Vec::new();
        let mut next_node = 0;
        let root = compile(&shape.0, &mut source, &mut transforms, &mut next_node)?;
        source.push_str(&format!(
            "return sdf_paint(n{root}, params.slots[0], params.slots[1]);\n}}\n"
        ));
        Ok(Self {
            shader: EffectShader::wgsl(source),
            transforms,
            options: Default::default(),
        })
    }
    /// Number of independently transformed leaves.
    pub fn shape_count(&self) -> usize {
        self.transforms.len()
    }
    /// Returns a leaf's current transform.
    pub fn transform(&self, index: usize) -> Option<SdfTransform> {
        self.transforms.get(index).copied()
    }
    /// Updates a leaf without rebuilding the shader. Returns false for an invalid index.
    /// Scale is limited to 0.001 through 1,000; non-finite values use neutral defaults.
    pub fn set_transform(&mut self, index: usize, transform: SdfTransform) -> bool {
        let Some(target) = self.transforms.get_mut(index) else {
            return false;
        };
        *target = normalize_transform(transform);
        true
    }
    pub fn options(&self) -> SdfOptions {
        self.options
    }
    /// Changes edge and lighting settings without rebuilding the shader.
    pub fn set_options(&mut self, options: SdfOptions) {
        self.options = options;
    }
    /// Reusable WGSL shader for this composition.
    pub fn shader(&self) -> EffectShader {
        self.shader.clone()
    }
    /// Packs the current transforms and style for a device scale.
    pub fn uniforms(&self, scale_factor: f32) -> EffectUniforms {
        let scale = finite(scale_factor, 1.).max(0.001);
        let options = self.options;
        let mut uniforms = EffectUniforms::new()
            .with_slot(
                0,
                [
                    scale,
                    nonnegative(options.smoothing.into()) * scale,
                    nonnegative(options.stroke_width.into()) * scale,
                    nonnegative(options.outer_glow.into()) * scale,
                ],
            )
            .with_slot(
                1,
                [
                    nonnegative(options.inner_glow.into()) * scale,
                    opacity(options.outer_glow_opacity),
                    opacity(options.inner_glow_intensity),
                    opacity(options.fill_opacity),
                ],
            );
        for (i, transform) in self.transforms.iter().enumerate() {
            uniforms.set_slot(
                i + 2,
                [
                    transform.center.x.into(),
                    transform.center.y.into(),
                    transform.scale,
                    transform.rotation,
                ],
            );
        }
        uniforms
    }
    /// Paints into a surface-local rectangle. Parent clipping and opacity are preserved.
    pub fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) -> Result<()> {
        window.paint_effect(
            PaintEffect::new(bounds, self.shader()).uniforms(self.uniforms(window.scale_factor())),
        )
    }
}
/// Builds a styled canvas for a distance field. Hit testing belongs to the canvas or its parent.
pub fn sdf(scene: &SdfScene) -> Canvas<()> {
    let scene = scene.clone();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let _ = scene.paint(bounds, window);
        },
    )
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}
fn nonnegative(value: f32) -> f32 {
    finite(value, 0.).max(0.)
}
fn opacity(value: f32) -> f32 {
    nonnegative(value).min(1.)
}
fn normalize_transform(t: SdfTransform) -> SdfTransform {
    SdfTransform {
        center: point(
            px(finite(t.center.x.into(), 0.)),
            px(finite(t.center.y.into(), 0.)),
        ),
        scale: finite(t.scale, 1.).clamp(0.001, 1000.),
        rotation: finite(t.rotation, 0.),
    }
}
fn compile(
    node: &Node,
    source: &mut String,
    transforms: &mut Vec<SdfTransform>,
    next: &mut usize,
) -> Result<usize> {
    let expression = match node {
        Node::Shape {
            geometry,
            color,
            transform,
        } => {
            anyhow::ensure!(
                transforms.len() < MAX_SDF_SHAPES,
                "SDF compositions support at most {MAX_SDF_SHAPES} shapes"
            );
            let slot = transforms.len() + 2;
            transforms.push(normalize_transform(*transform));
            let p = format!("sdf_local(position, params.slots[{slot}], params.slots[0].x)");
            let distance = match geometry {
                Geometry::Circle(radius) => format!("length({p}) - {radius:?}"),
                Geometry::RoundedRect(half, radius) => format!(
                    "sdf_round_rect({p}, vec2<f32>({:?}, {:?}), {radius:?})",
                    half[0], half[1]
                ),
                Geometry::Capsule(length, radius) => {
                    format!("sdf_capsule({p}, {length:?}, {radius:?})")
                }
            };
            let alpha = opacity(color.a);
            format!(
                "SdfSample(({distance}) * params.slots[{slot}].z * params.slots[0].x, vec4<f32>({:?}, {:?}, {:?}, {alpha:?}))",
                opacity(color.r) * alpha,
                opacity(color.g) * alpha,
                opacity(color.b) * alpha
            )
        }
        Node::Combine {
            left,
            right,
            operation,
            smooth,
        } => {
            let left = compile(left, source, transforms, next)?;
            let right = compile(right, source, transforms, next)?;
            let function = match operation {
                Operation::Union => "sdf_union",
                Operation::Intersect => "sdf_intersect",
                Operation::Subtract => "sdf_subtract",
            };
            format!(
                "{function}(n{left}, n{right}, {})",
                if *smooth { "params.slots[0].y" } else { "0.0" }
            )
        }
    };
    let id = *next;
    *next += 1;
    source.push_str(&format!("let n{id} = {expression};\n"));
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composed_sdf_shaders_validate_and_transform_without_recompilation() {
        let a = SdfShape::circle(point(px(20.), px(20.)), px(12.), gpui::rgb(0x50cfff));
        let b = SdfShape::rounded_rect(
            point(px(38.), px(20.)),
            gpui::size(px(20.), px(16.)),
            px(4.),
            gpui::rgb(0xa060ff),
        );
        let c = SdfShape::capsule(
            point(px(30.), px(30.)),
            px(12.),
            px(3.),
            gpui::rgb(0xff9070),
        );
        for root in [
            a.clone().union(b.clone()).subtract(c.clone()),
            a.clone().smooth_union(b.clone()).smooth_subtract(c),
            a.clone().intersect(b.clone()),
            a.smooth_intersect(b),
        ] {
            let mut scene = SdfScene::new(root).unwrap();
            let shader = scene.shader();
            let source = gpui::compose_effect_shader_wgsl(&shader);
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
            let before = scene.uniforms(1.);
            scene.set_transform(
                0,
                SdfTransform {
                    center: point(px(80.), px(50.)),
                    scale: 1.5,
                    rotation: 0.6,
                },
            );
            assert_ne!(before.slots()[2], scene.uniforms(1.).slots()[2]);
            assert_eq!(shader.id(), scene.shader().id());
        }
    }

    #[test]
    fn shape_limit_rejects_uniform_overflow() {
        let leaf = SdfShape::circle(point(px(0.), px(0.)), px(10.), gpui::rgb(0xffffff));
        let mut root = leaf.clone();
        for _ in 1..MAX_SDF_SHAPES {
            root = root.smooth_union(leaf.clone());
        }
        let scene = SdfScene::new(root.clone()).unwrap();
        assert!(scene.transform(MAX_SDF_SHAPES - 1).is_some());
        assert!(SdfScene::new(root.union(leaf)).is_err());
    }
}
