use crate::{Hit, Scene, Texture};
use gpui::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, MeshDraw3d, MeshTexture3d, Pixels, PointerTransform, Scene3dFrame,
    Style, StyleRefinement, Styled, Window, div, prelude::*,
};
use std::{cell::Cell, rc::Rc, sync::Arc};

type HoverListener = Box<dyn Fn(&Option<Hit>, &mut Window, &mut App)>;
type ClickListener = Box<dyn Fn(&Hit, &mut Window, &mut App)>;

/// Creates a layout-sized 3D viewport. The caller owns camera interaction and animation.
pub fn viewport3d(id: impl Into<ElementId>, scene: Scene) -> Viewport3d {
    Viewport3d {
        id: id.into(),
        scene,
        texture: None,
        style: StyleRefinement::default(),
        on_hover: None,
        on_click: None,
        pick_bounds: Rc::new(Cell::new(None)),
    }
}

/// A styled viewport with one optional decorative UI texture.
pub struct Viewport3d {
    id: ElementId,
    scene: Scene,
    texture: Option<AnyElement>,
    style: StyleRefinement,
    on_hover: Option<HoverListener>,
    on_click: Option<ClickListener>,
    pick_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}
impl Viewport3d {
    /// Reports geometric hits on pointer movement and `None` on exit or a miss.
    /// Texture alpha is not sampled. This does not request animation frames.
    pub fn on_object_hover(
        mut self,
        listener: impl Fn(&Option<Hit>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_hover = Some(Box::new(listener));
        self
    }
    /// Handles a left click whose endpoints hit the same mesh within four logical pixels.
    /// Texture alpha is not sampled. Callers sharing the button with camera gestures
    /// should ignore clicks after a drag.
    pub fn on_object_click(
        mut self,
        listener: impl Fn(&Hit, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(listener));
        self
    }
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
        if let Some(listener) = self.on_hover.take() {
            let listener = Rc::new(listener);
            let scene = self.scene.clone();
            let bounds = self.pick_bounds.clone();
            let on_move = listener.clone();
            container = container
                .on_mouse_move(move |event, window, cx| {
                    let hit = bounds
                        .get()
                        .and_then(|bounds| scene.pick(bounds, event.position));
                    on_move(&hit, window, cx);
                })
                .on_hover(move |hovered, window, cx| {
                    if !hovered {
                        listener(&None, window, cx);
                    }
                });
        }
        if let Some(listener) = self.on_click.take() {
            let scene = self.scene.clone();
            let bounds = self.pick_bounds.clone();
            container = container.on_click(move |event, window, cx| {
                let gpui::ClickEvent::Mouse(event) = event else {
                    return;
                };
                let delta = event.up.position - event.down.position;
                if f32::from(delta.x).hypot(f32::from(delta.y)) > 4. {
                    return;
                }
                let Some(bounds) = bounds.get() else {
                    return;
                };
                let Some(down) = scene.pick(bounds, event.down.position) else {
                    return;
                };
                let Some(up) = scene.pick(bounds, event.up.position) else {
                    return;
                };
                if down.object_index == up.object_index {
                    listener(&up, window, cx);
                }
            });
        }
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
        self.0.pick_bounds.set(Some(bounds));
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
