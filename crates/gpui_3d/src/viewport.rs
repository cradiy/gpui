use crate::{Scene, Texture};
use gpui::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, MeshDraw3d, MeshTexture3d, Pixels, PointerTransform, Scene3dFrame,
    Style, StyleRefinement, Styled, Window, div, prelude::*,
};
use std::sync::Arc;

/// Creates a layout-sized 3D viewport. The caller owns camera interaction and animation.
pub fn viewport3d(id: impl Into<ElementId>, scene: Scene) -> Viewport3d {
    Viewport3d {
        id: id.into(),
        scene,
        texture: None,
        style: StyleRefinement::default(),
    }
}

/// A styled viewport with one optional decorative UI texture.
pub struct Viewport3d {
    id: ElementId,
    scene: Scene,
    texture: Option<AnyElement>,
    style: StyleRefinement,
}
impl Viewport3d {
    /// Captures UI at viewport layout size for every `Material::ui()` object.
    /// This content is visual only; mesh-to-UI input routing is not provided.
    pub fn ui_texture(mut self, content: impl IntoElement) -> Self {
        self.texture = Some(
            div()
                .id("ui-texture")
                .absolute()
                .inset_0()
                .size_full()
                .child(content)
                .into_any_element(),
        );
        self
    }
}
impl Styled for Viewport3d {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
impl IntoElement for Viewport3d {
    type Element = gpui::Stateful<gpui::Div>;
    fn into_element(mut self) -> Self::Element {
        let mut container = div().id(self.id.clone()).relative();
        container.style().refine(&std::mem::take(&mut self.style));
        container.child(Content(self))
    }
}
struct Content(Viewport3d);
impl IntoElement for Content {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Content {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        Some("scene".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        for object in &self.0.scene.objects {
            if let Texture::Image(image) = &object.material.texture {
                let _ = image.use_data(None, window, cx);
            }
        }
        let children = self
            .0
            .texture
            .iter_mut()
            .map(|child| child.request_layout(window, cx))
            .collect::<Vec<_>>();
        (
            window.request_layout(
                Style {
                    size: gpui::size(gpui::relative(1.).into(), gpui::relative(1.).into()),
                    ..Default::default()
                },
                children,
                cx,
            ),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if window.supports_scene3d() {
            window.prepaint_subtree_effect(|window| {
                window.with_pointer_transform(
                    bounds,
                    PointerTransform::noninteractive(),
                    |window| {
                        if let Some(texture) = &mut self.0.texture {
                            texture.prepaint(window, cx);
                        }
                    },
                );
            });
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !window.supports_scene3d() || bounds.is_empty() {
            return;
        }
        let scene = &self.0.scene;
        let objects = scene
            .objects
            .iter()
            .filter_map(|object| {
                let texture = match &object.material.texture {
                    Texture::None => MeshTexture3d::None,
                    Texture::Ui => MeshTexture3d::Subtree,
                    Texture::Image(source) => {
                        let image = source.use_data(None, window, cx)?.ok()?;
                        MeshTexture3d::Image(window.prepare_effect_image(&image, 0).ok()?)
                    }
                };
                let (model, normal) = object.transform.matrices();
                Some(MeshDraw3d {
                    mesh: object.mesh.0.clone(),
                    model,
                    normal,
                    color: object.material.color,
                    texture,
                    alpha_cutoff: object.material.alpha_cutoff,
                    unlit: object.material.unlit,
                })
            })
            .collect::<Vec<_>>();
        let light = scene.light;
        assert!(
            light
                .direction
                .iter()
                .chain([light.intensity, light.ambient].iter())
                .all(|x| x.is_finite())
        );
        let frame = Arc::new(Scene3dFrame {
            view_projection: scene
                .camera
                .matrix(f32::from(bounds.size.width) / f32::from(bounds.size.height)),
            light_direction: light.direction,
            light: [
                light.color.r,
                light.color.g,
                light.color.b,
                light.intensity.max(0.),
            ],
            ambient: light.ambient.max(0.),
            objects: objects.into(),
        });
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.with_scene3d(bounds, frame, |window| {
                if let Some(texture) = &mut self.0.texture {
                    texture.paint(window, cx);
                }
            });
        });
    }
}
