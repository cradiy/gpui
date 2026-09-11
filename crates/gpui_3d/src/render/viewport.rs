use super::ui_input::UiInput;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
use super::viewport_picking::PickLayout;
use crate::spatial::picking::{PickSnapshot, PickSurface};
use crate::{
    Hit, ObjectId, PickBehavior, PreparationCache, PreparedScene, Scene, Texture, TextureSlot,
    TextureSource, TextureState,
};
use gpui::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, MeshTexture3d, Pixels, PointerTransform, Size, Style, StyleRefinement,
    Styled, UiTexture3d, Window, div, prelude::*,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

type HoverListener = Box<dyn Fn(&Option<Hit>, &mut Window, &mut App)>;
type ClickListener = Box<dyn Fn(&Hit, &mut Window, &mut App)>;

#[derive(Default)]
struct ViewportPreparation {
    cache: PreparationCache,
    output: Option<(Arc<PreparedScene>, Arc<gpui::Scene3dFrame>)>,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pick_binding: Option<(crate::ViewportPickCapture, PickLayout)>,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pick_owner: Rc<()>,
}

/// Creates a layout-sized 3D viewport. The caller owns camera interaction and animation.
pub fn viewport3d(id: impl Into<ElementId>, scene: Scene) -> Viewport3d {
    Viewport3d {
        id: id.into(),
        scene,
        texture: None,
        texture_size: None,
        texture_scale: 1.,
        quality: Default::default(),
        interactive_ui: None,
        style: StyleRefinement::default(),
        on_hover: None,
        on_click: None,
        pick_snapshot: Rc::new(RefCell::new(None)),
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        rendered_picks: None,
    }
}

/// A styled viewport with one optional UI texture.
pub struct Viewport3d {
    id: ElementId,
    scene: Scene,
    texture: Option<AnyElement>,
    texture_size: Option<Size<Pixels>>,
    texture_scale: f32,
    quality: crate::ViewportQuality,
    interactive_ui: Option<ObjectId>,
    style: StyleRefinement,
    on_hover: Option<HoverListener>,
    on_click: Option<ClickListener>,
    pick_snapshot: Rc<RefCell<Option<PickSnapshot>>>,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    rendered_picks: Option<crate::ViewportPickCapture>,
}
impl Viewport3d {
    /// Publishes ID/depth queries paired with this viewport's submitted frame.
    /// Reuse a dedicated capture across renders. Does not enable CPU/UI hit routing
    /// or schedule readback polling; query the capture from input handlers.
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pub fn pick_capture(mut self, capture: crate::ViewportPickCapture) -> Self {
        self.rendered_picks = Some(capture);
        self
    }

    /// Draws packed geometry without vertex readback. IDs are scene object indices plus one.
    /// Create resources with `WgpuContext::for_window`; the renderer rejects wrong-device,
    /// source-mesh, and material-coordinate bindings. Bounds are conservative and mesh-local.
    /// GPU overrides disable CPU object picking and captured-UI pointer routing for this
    /// viewport. Materialize CPU meshes to use those interactions at the deformed pose.
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pub fn gpu_geometry(mut self, draws: &[crate::Scene3dGpuDraw]) -> anyhow::Result<Self> {
        self.scene = super::gpu_geometry::with_geometry(&self.scene, draws)?;
        Ok(self)
    }

    /// Sets mesh raster density relative to physical render-surface pixels. Defaults to one.
    /// Must be finite and positive; dimensions are capped uniformly by the device.
    /// Does not change layout, camera projection, picking, or UI capture density.
    #[track_caller]
    pub fn resolution_scale(mut self, scale: f32) -> Self {
        self.quality = crate::ViewportQuality::new(scale, self.quality.color_samples());
        self
    }

    /// Requests one or four color samples. Defaults to four, with a fallback to
    /// one when unsupported by the current backend. Does not change input coordinates.
    #[track_caller]
    pub fn color_samples(mut self, samples: u32) -> Self {
        self.quality = crate::ViewportQuality::new(self.quality.resolution_scale(), samples);
        self
    }

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
    /// Defaults to 1. Density is capped uniformly at 2048 pixels on either axis
    /// and at the current renderer's UI texture limit.
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
                    if !window.supports_scene3d() {
                        on_move(&None, window, cx);
                        return;
                    }
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
                if !window.supports_scene3d() {
                    return;
                }
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
            for (_, map) in object.material.lighting_textures() {
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
        let capabilities = window.scene3d_support().capabilities();
        if let Some(input) = &input {
            input.read(cx).prepare(
                target.filter(|_| {
                    capabilities.is_some()
                        && self.0.texture.is_some()
                        && !bounds.is_empty()
                        && cpu_interaction_enabled(&self.0.scene)
                }),
                self.0.texture_size.unwrap_or(bounds.size),
                window,
            );
        }
        if let Some(capabilities) = capabilities
            && !bounds.is_empty()
            && self.0.texture.is_some()
        {
            let logical_size = self.0.texture_size.unwrap_or(bounds.size);
            let limit = capabilities.max_ui_texture_dimension as f32
                / f32::from(logical_size.width.max(logical_size.height));
            let config = UiTexture3d::new(
                logical_size,
                (window.scale_factor() * self.0.texture_scale).min(limit),
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
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        texture_state: &mut Option<TexturePrepaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !window.supports_scene3d() || bounds.is_empty() {
            *self.0.pick_snapshot.borrow_mut() = None;
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            if let Some(capture) = &self.0.rendered_picks {
                capture.clear();
            }
            return;
        }
        let scene = &self.0.scene;
        let mut surfaces: Vec<_> = scene.objects.iter().map(|_| PickSurface::Absent).collect();
        let has_ui = self.0.texture.is_some();
        let (prepared, frame) =
            window.with_element_state(id.unwrap(), |state: Option<ViewportPreparation>, window| {
                let mut state = state.unwrap_or_default();
                #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                let layout = PickLayout {
                    bounds,
                    scale: window.scale_factor(),
                    surface: window.viewport_size(),
                };
                #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                let binding_matches = match (&state.pick_binding, &self.0.rendered_picks) {
                    (None, None) => true,
                    (Some((old, old_layout)), Some(capture)) => {
                        old.same(capture) && *old_layout == layout
                    }
                    _ => false,
                };
                #[cfg(not(all(feature = "wgpu", not(target_family = "wasm"))))]
                let binding_matches = true;
                let prepared = state
                    .cache
                    .prepare(
                        scene,
                        f32::from(bounds.size.width) / f32::from(bounds.size.height),
                        texture_state.as_ref().map(|state| state.config),
                        |request| {
                            let mut surface = PickSurface::Absent;
                            let texture = match request.source {
                                TextureSource::Solid => {
                                    surface = PickSurface::Solid;
                                    MeshTexture3d::None
                                }
                                TextureSource::Ui => {
                                    if has_ui {
                                        surface = PickSurface::Solid;
                                    }
                                    MeshTexture3d::Subtree
                                }
                                TextureSource::Image(source) => {
                                    let Some(Ok(image)) = source.use_data(None, window, cx) else {
                                        return Ok(TextureState::Pending);
                                    };
                                    let Ok(tile) = window.prepare_effect_image(&image, 0) else {
                                        return Ok(TextureState::Pending);
                                    };
                                    surface = PickSurface::Image(image);
                                    MeshTexture3d::Image(tile)
                                }
                            };
                            if request.slot == TextureSlot::BaseColor {
                                surfaces[request.object_index] = surface;
                            }
                            Ok(TextureState::Ready(texture))
                        },
                    )
                    .expect("invalid 3D scene");
                let frame = state
                    .output
                    .as_ref()
                    .filter(|(previous, frame)| {
                        binding_matches
                            && Arc::ptr_eq(previous, &prepared)
                            && frame.viewport_quality == self.0.quality
                    })
                    .map(|(_, frame)| frame.clone())
                    .unwrap_or_else(|| {
                        let mut frame = prepared.frame().clone();
                        frame.viewport_quality = self.0.quality;
                        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                        {
                            frame.pick_capture = self
                                .0
                                .rendered_picks
                                .as_ref()
                                .map(|capture| capture.backend());
                        }
                        Arc::new(frame)
                    });
                state.output = Some((prepared.clone(), frame.clone()));
                #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                {
                    if !binding_matches && let Some((old, _)) = &state.pick_binding {
                        old.clear();
                    }
                    state.pick_binding = self.0.rendered_picks.as_ref().map(|capture| {
                        capture.bind(
                            frame.clone(),
                            &prepared,
                            scene.camera,
                            layout,
                            &state.pick_owner,
                        );
                        (capture.clone(), layout)
                    });
                }
                ((prepared, frame), state)
            });
        *self.0.pick_snapshot.borrow_mut() = pick_snapshot(scene, bounds, surfaces, &prepared);
        if let Some(state) = texture_state
            && let Some(input) = &state.input
        {
            input.read(cx).set_snapshot(self.0.pick_snapshot.clone());
            input.read(cx).paint(&state.hitbox, window);
        }
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

fn cpu_interaction_enabled(scene: &Scene) -> bool {
    scene
        .objects
        .iter()
        .all(|object| object.gpu_geometry.is_none() && object.material.custom_material.is_none())
}

fn pick_snapshot(
    scene: &Scene,
    bounds: Bounds<Pixels>,
    mut surfaces: Vec<PickSurface>,
    prepared: &PreparedScene,
) -> Option<PickSnapshot> {
    if !cpu_interaction_enabled(scene) {
        return None;
    }
    for pending in prepared.pending_textures() {
        surfaces[pending.object_index] = PickSurface::Absent;
    }
    Some(PickSnapshot {
        scene: scene.clone(),
        bounds,
        surfaces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Material, MaterialTexture, Mesh, Object, PbrMaterial};
    use gpui::{DevicePixels, point, px, rgb, size};

    #[test]
    fn gpu_geometry_disables_cpu_hits_without_changing_source_queries() {
        let source =
            Scene::new().object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))));
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)));
        let prepared = PreparationCache::new()
            .prepare(&source, 1., None, |_| {
                Ok(TextureState::Ready(MeshTexture3d::None))
            })
            .unwrap();
        assert!(
            pick_snapshot(&source, bounds, vec![PickSurface::Solid], &prepared)
                .unwrap()
                .pick(point(px(50.), px(50.)))
                .is_some()
        );
        let mut render_scene = source.clone();
        render_scene.objects[0].gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(Arc::new(())));
        assert!(
            pick_snapshot(&render_scene, bounds, vec![PickSurface::Solid], &prepared).is_none()
        );
        assert!(cpu_interaction_enabled(&source));
        assert!(!cpu_interaction_enabled(&render_scene));
        render_scene.objects[0].gpu_geometry = None;
        assert!(
            pick_snapshot(&render_scene, bounds, vec![PickSurface::Solid], &prepared)
                .unwrap()
                .pick(point(px(50.), px(50.)))
                .is_some()
        );
    }

    #[test]
    fn custom_material_disables_cpu_coverage_until_restored() {
        let mut source =
            Scene::new().object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))));
        assert!(cpu_interaction_enabled(&source));
        source.objects[0].material.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(())));
        let retained = source.clone();
        assert!(!cpu_interaction_enabled(&source));
        source.objects[0].material = source.objects[0].material.clone().builtin_program();
        assert!(cpu_interaction_enabled(&source));
        assert!(!cpu_interaction_enabled(&retained));
    }

    #[test]
    fn viewport_quality_validates_requests_and_resolves_device_sampling() {
        for (scale, samples) in [
            (0., 4),
            (-1., 4),
            (f32::NAN, 4),
            (f32::INFINITY, 4),
            (1., 0),
            (1., 2),
            (1., 8),
        ] {
            assert!(
                std::panic::catch_unwind(|| crate::ViewportQuality::new(scale, samples)).is_err()
            );
        }
        for supported in [1, 4] {
            let capabilities = gpui::Scene3dViewportCapabilities {
                max_texture_dimension: 4096,
                color_samples: supported,
                max_ui_texture_dimension: 2048,
            };
            for requested in [1, 4] {
                let view = viewport3d("quality", Scene::new())
                    .resolution_scale(0.5)
                    .color_samples(requested);
                assert_eq!(view.quality.resolution_scale(), 0.5);
                assert_eq!(view.quality.color_samples(), requested);
                assert_eq!(
                    capabilities.color_samples_for(view.quality),
                    supported.min(requested)
                );
                assert_eq!(view.texture_scale, 1.);
            }
        }
    }

    #[test]
    fn pending_lighting_maps_do_not_leave_invisible_pick_surfaces() {
        let material = Material::color(rgb(0xffffff))
            .pbr(PbrMaterial::default())
            .normal_texture(MaterialTexture::new("normal.png"));
        let scene = Scene::new()
            .object(
                Object::new(Mesh::plane(), material)
                    .position([0., 0., 1.])
                    .id("front"),
            )
            .object(Object::new(Mesh::plane(), Material::color(rgb(0x80a0c0))).id("back"));
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)));
        let mut cache = PreparationCache::new();
        for ready in [false, false, true, true, false] {
            let prepared = cache
                .prepare(&scene, 1., None, |request| {
                    Ok(if request.slot == TextureSlot::Normal {
                        if ready {
                            TextureState::Ready(MeshTexture3d::Image(gpui::AtlasTile {
                                texture_id: gpui::AtlasTextureId {
                                    index: 0,
                                    kind: gpui::AtlasTextureKind::Polychrome,
                                },
                                tile_id: gpui::TileId(0),
                                padding: 0,
                                bounds: Bounds::new(
                                    point(DevicePixels(0), DevicePixels(0)),
                                    size(DevicePixels(1), DevicePixels(1)),
                                ),
                            }))
                        } else {
                            TextureState::Pending
                        }
                    } else {
                        TextureState::Ready(MeshTexture3d::None)
                    })
                })
                .unwrap();
            let snapshot = pick_snapshot(
                &scene,
                bounds,
                vec![PickSurface::Solid, PickSurface::Solid],
                &prepared,
            );
            let hit = snapshot.unwrap().pick(point(px(50.), px(50.))).unwrap();
            assert_eq!(
                hit.object_id,
                Some(if ready { "front" } else { "back" }.into())
            );
        }
    }
}
