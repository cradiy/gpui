use crate::picking::{PickSnapshot, PickSurface};
use crate::{Hit, ObjectId, PickBehavior, Scene, Texture, TextureSlot, ui_input::UiInput};
use gpui::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, MeshTexture3d, Pixels, PointerTransform, Size, Style, StyleRefinement,
    Styled, UiTexture3d, Window, div, prelude::*,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

type HoverListener = Box<dyn Fn(&Option<Hit>, &mut Window, &mut App)>;
type ClickListener = Box<dyn Fn(&Hit, &mut Window, &mut App)>;

/// Creates a layout-sized 3D viewport. The caller owns camera interaction and animation.
pub fn viewport3d(id: impl Into<ElementId>, scene: Scene) -> Viewport3d {
    Viewport3d {
        id: id.into(),
        scene,
        texture: None,
        texture_size: None,
        texture_scale: 1.,
        interactive_ui: None,
        style: StyleRefinement::default(),
        on_hover: None,
        on_click: None,
        pick_snapshot: Rc::new(RefCell::new(None)),
    }
}

/// A styled viewport with one optional UI texture.
pub struct Viewport3d {
    id: ElementId,
    scene: Scene,
    texture: Option<AnyElement>,
    texture_size: Option<Size<Pixels>>,
    texture_scale: f32,
    interactive_ui: Option<ObjectId>,
    style: StyleRefinement,
    on_hover: Option<HoverListener>,
    on_click: Option<ClickListener>,
    pick_snapshot: Rc<RefCell<Option<PickSnapshot>>>,
}
impl Viewport3d {
    /// Reports hits on pointer movement and `None` on exit or a miss.
    /// Image alpha is sampled; captured UI uses mesh geometry. No frames are scheduled.
    pub fn on_object_hover(
        mut self,
        listener: impl Fn(&Option<Hit>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_hover = Some(Box::new(listener));
        self
    }
    /// Handles a left click whose endpoints hit the same mesh within four logical pixels.
    /// Image alpha is sampled; captured UI uses mesh geometry. Callers sharing the button with camera gestures
    /// should ignore clicks after a drag.
    pub fn on_object_click(
        mut self,
        listener: impl Fn(&Hit, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(listener));
        self
    }
    /// Captures UI for every `Material::ui()` object.
    /// Content is decorative unless `interactive_ui` enables pointer routing.
    pub fn ui_texture(mut self, content: impl IntoElement) -> Self {
        self.texture = Some(
            div()
                .id("ui-texture")
                .size_full()
                .overflow_hidden()
                .child(content)
                .into_any_element(),
        );
        self
    }

    /// Sets the UI's logical layout dimensions. Defaults to the viewport size.
    #[track_caller]
    pub fn ui_texture_size(mut self, size: Size<Pixels>) -> Self {
        let _ = UiTexture3d::new(size, 1.);
        self.texture_size = Some(size);
        self
    }

    /// Sets raster density relative to display scale without changing layout.
    /// Defaults to 1. Density is capped uniformly at 2048 pixels on either axis.
    #[track_caller]
    pub fn ui_texture_scale(mut self, scale: f32) -> Self {
        assert!(scale.is_finite() && scale > 0.);
        self.texture_scale = scale;
        self
    }

    /// Routes pointer input into the UI texture on one uniquely named UI object.
    /// Left-button gestures and scrolling on this surface do not bubble to camera
    /// controls. Right-button gestures remain available for orbit interaction.
    pub fn interactive_ui(mut self, object: impl Into<ObjectId>) -> Self {
        self.interactive_ui = Some(object.into());
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
            let snapshot = self.pick_snapshot.clone();
            let on_move = listener.clone();
            container = container
                .on_mouse_move(move |event, window, cx| {
                    let hit = snapshot
                        .borrow()
                        .as_ref()
                        .and_then(|snapshot| snapshot.pick(event.position));
                    on_move(&hit, window, cx);
                })
                .on_hover(move |hovered, window, cx| {
                    if !hovered {
                        listener(&None, window, cx);
                    }
                });
        }
        if let Some(listener) = self.on_click.take() {
            let snapshot = self.pick_snapshot.clone();
            container = container.on_click(move |event, window, cx| {
                let gpui::ClickEvent::Mouse(event) = event else {
                    return;
                };
                let delta = event.up.position - event.down.position;
                if f32::from(delta.x).hypot(f32::from(delta.y)) > 4. {
                    return;
                }
                let hit = snapshot.borrow().as_ref().and_then(|snapshot| {
                    let down = snapshot.pick(event.down.position)?;
                    let up = snapshot.pick(event.up.position)?;
                    (down.object_index == up.object_index).then_some(up)
                });
                if let Some(up) = hit {
                    listener(&up, window, cx);
                }
            });
        }
        container.child(Content(self))
    }
}
struct Content(Viewport3d);
struct TexturePrepaint {
    config: UiTexture3d,
    transform: PointerTransform,
    input: Option<gpui::Entity<UiInput>>,
    hitbox: gpui::Hitbox,
}
impl IntoElement for Content {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Content {
    type RequestLayoutState = ();
    type PrepaintState = Option<TexturePrepaint>;
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
            for (_, map) in object.material.pbr_textures() {
                let _ = map.image.use_data(None, window, cx);
            }
        }
        (
            window.request_layout(
                Style {
                    size: gpui::size(gpui::relative(1.).into(), gpui::relative(1.).into()),
                    ..Default::default()
                },
                [],
                cx,
            ),
            (),
        )
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<TexturePrepaint> {
        let target = self
            .0
            .interactive_ui
            .as_ref()
            .filter(|target| {
                let matches = self
                    .0
                    .scene
                    .objects
                    .iter()
                    .filter(|object| {
                        object.id.as_ref() == Some(target)
                            && matches!(object.material.texture, Texture::Ui)
                            && object.pick_behavior == PickBehavior::Target
                    })
                    .count();
                assert!(matches <= 1, "interactive UI requires a unique object ID");
                matches == 1
            })
            .cloned();
        let input = window.with_element_state(
            id.unwrap(),
            |state: Option<Option<gpui::Entity<UiInput>>>, window| {
                let input = state.flatten().or_else(|| {
                    self.0
                        .interactive_ui
                        .as_ref()
                        .map(|_| cx.new(|cx| UiInput::new(window, cx)))
                });
                (input.clone(), input)
            },
        );
        if let Some(input) = &input {
            input.read(cx).prepare(
                target.filter(|_| self.0.texture.is_some() && !bounds.is_empty()),
                self.0.texture_size.unwrap_or(bounds.size),
                window,
            );
        }
        if window.supports_scene3d() && !bounds.is_empty() && self.0.texture.is_some() {
            let config = UiTexture3d::new(
                self.0.texture_size.unwrap_or(bounds.size),
                window.scale_factor() * self.0.texture_scale,
            );
            let transform = input
                .as_ref()
                .map_or_else(PointerTransform::noninteractive, |input| {
                    input.read(cx).transform()
                });
            let hitbox = window.prepaint_subtree_effect(|window| {
                window.with_pointer_transform(bounds, transform.clone(), |window| {
                    window.with_scene3d_texture(config, |window| {
                        let hitbox = window.insert_hitbox(
                            Bounds::new(Default::default(), config.logical_size()),
                            gpui::HitboxBehavior::Normal,
                        );
                        self.0.texture.as_mut().unwrap().prepaint_as_root(
                            Default::default(),
                            config.logical_size().into(),
                            window,
                            cx,
                        );
                        hitbox
                    })
                })
            });
            Some(TexturePrepaint {
                config,
                transform,
                input,
                hitbox,
            })
        } else {
            None
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        texture_state: &mut Option<TexturePrepaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !window.supports_scene3d() || bounds.is_empty() {
            return;
        }
        let scene = &self.0.scene;
        let mut surfaces: Vec<_> = scene.objects.iter().map(|_| PickSurface::Absent).collect();
        let has_ui = self.0.texture.is_some();
        let frame = scene
            .prepare_frame(
                f32::from(bounds.size.width) / f32::from(bounds.size.height),
                texture_state.as_ref().map(|state| state.config),
                |index, slot, source| {
                    let mut surface = PickSurface::Absent;
                    let texture = match source {
                        Texture::None => {
                            surface = PickSurface::Solid;
                            MeshTexture3d::None
                        }
                        Texture::Ui => {
                            if has_ui {
                                surface = PickSurface::Solid;
                            }
                            MeshTexture3d::Subtree
                        }
                        Texture::Image(source) => {
                            let Some(Ok(image)) = source.use_data(None, window, cx) else {
                                return Ok(None);
                            };
                            let Ok(tile) = window.prepare_effect_image(&image, 0) else {
                                return Ok(None);
                            };
                            surface = PickSurface::Image(image);
                            MeshTexture3d::Image(tile)
                        }
                    };
                    if slot == TextureSlot::BaseColor {
                        surfaces[index] = surface;
                    }
                    Ok(Some(texture))
                },
            )
            .expect("invalid 3D scene");
        *self.0.pick_snapshot.borrow_mut() = Some(PickSnapshot {
            scene: scene.clone(),
            bounds,
            surfaces,
        });
        if let Some(state) = texture_state
            && let Some(input) = &state.input
        {
            input.read(cx).set_snapshot(self.0.pick_snapshot.clone());
            input.read(cx).paint(&state.hitbox, window);
        }
        let frame = Arc::new(frame);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.with_scene3d(bounds, frame, |window| {
                if let Some(state) = texture_state {
                    window.with_pointer_transform(bounds, state.transform.clone(), |window| {
                        window.with_scene3d_texture(state.config, |window| {
                            self.0.texture.as_mut().unwrap().paint(window, cx);
                        });
                    });
                }
            });
        });
    }
}
