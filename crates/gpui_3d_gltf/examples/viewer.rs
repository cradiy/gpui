use std::{
    cell::Cell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Context as _, Result};
use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Subscription, Task, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, EvaluatedScene, NodeHandle, OrbitController, Projection, Scene, SceneGraph, viewport3d,
};
use gpui_3d_gltf::{
    DecodedScene, Document, ImageCache, ImageDecodeLimits, Limits, SceneAsset, SceneInstance,
    SceneLoadCompletion, SceneLoadQueue, SceneLoadSlot, SceneLoadStatus, SceneOptions,
};
use gpui_platform::application;

#[path = "support/files.rs"]
mod files;

const USAGE: &str = "Usage: viewer <asset.gltf|asset.glb> [SCENE_INDEX]";

struct PixelsReady {
    decoded: DecodedScene,
    materials: HashMap<Option<usize>, String>,
}

fn fitted_size(available: gpui::Size<Pixels>, aspect: Option<f32>) -> gpui::Size<Pixels> {
    let Some(aspect) = aspect else {
        return available;
    };
    let width = available.width.min(available.height * aspect);
    size(width, width / aspect)
}

async fn load(path: &Path, scene: Option<usize>, images: &ImageCache) -> Result<PixelsReady> {
    let limits = Limits::default();
    let path = path.canonicalize().context("asset path")?;
    let root = path.parent().context("asset has no directory")?;
    let document =
        Document::from_slice(&files::read_bounded(&path, limits.document_bytes)?, limits)?;
    let materials = document.gltf().materials().map(|material| {
        let pbr = material.pbr_metallic_roughness();
        (material.index(), format!("{}\nBase color {:?}\nMetallic {:.2} · Roughness {:.2}\nAlpha {:?} · Double sided {}",
            material.name().unwrap_or("Unnamed material"), pbr.base_color_factor(),
            pbr.metallic_factor(), pbr.roughness_factor(), material.alpha_mode(), material.double_sided()))
    }).collect();
    let prepared = document
        .prepare_async(|request| {
            let result = files::resource_path(root, &request.uri)
                .and_then(|path| files::read_bounded(&path, request.byte_limit));
            std::future::ready(result)
        })
        .await?;
    let decoded = prepared
        .scene(scene, SceneOptions::default())?
        .decode_resources_cached(images, ImageDecodeLimits::default())?;
    Ok(PixelsReady { decoded, materials })
}

struct Model {
    instance: SceneInstance,
    evaluated: EvaluatedScene,
    materials: HashMap<Option<usize>, String>,
}

fn publish(
    slot: &mut SceneLoadSlot,
    model: &mut Option<Model>,
    completion: SceneLoadCompletion<PixelsReady>,
) -> bool {
    let mut replacement = None;
    let accepted = slot.accept_with(completion, |pixels| {
        let asset = pixels.decoded.resolve()?;
        let mut graph = SceneGraph::new();
        let instance = asset.instantiate(&mut graph, None)?;
        let evaluated = graph.evaluate()?;
        replacement = Some(Model {
            instance,
            evaluated,
            materials: pixels.materials,
        });
        Ok(asset)
    });
    if let Some(next) = replacement {
        *model = Some(next);
    }
    accepted && slot.status() == SceneLoadStatus::Ready
}

impl Model {
    fn asset(&self) -> &SceneAsset {
        self.instance.asset()
    }

    fn details(&self, selected: Option<NodeHandle>) -> Vec<String> {
        let Some(primitive) = selected.and_then(|node| self.instance.source_primitive(node)) else {
            return vec![
                format!("Scene {}", self.asset().index()),
                format!(
                    "{} nodes · {} primitives",
                    self.asset().nodes().len(),
                    self.asset().primitives().len()
                ),
                format!(
                    "{} skins · {} morph bindings",
                    self.asset().skins().len(),
                    self.asset().morphs().len()
                ),
                "Click a surface to inspect its source node and material.".into(),
            ];
        };
        let node = self
            .asset()
            .nodes()
            .iter()
            .find(|node| node.index == primitive.node_index);
        vec![
            format!(
                "Node {} · {}",
                primitive.node_index,
                node.and_then(|n| n.name.as_deref()).unwrap_or("Unnamed")
            ),
            format!(
                "Mesh {} · Primitive {}",
                primitive.mesh_index, primitive.primitive_index
            ),
            format!("Material {:?}", primitive.material_index),
            self.materials
                .get(&primitive.material_index)
                .cloned()
                .unwrap_or_else(|| "Default material".into()),
        ]
    }
}

struct Viewer {
    path: PathBuf,
    scene_index: Option<usize>,
    slot: SceneLoadSlot,
    model: Option<Model>,
    images: ImageCache,
    queue: SceneLoadQueue,
    task: Option<Task<()>>,
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    frame_pending: bool,
    selected: Option<NodeHandle>,
    source_camera: Option<NodeHandle>,
    error: Option<String>,
    _activation: Subscription,
}

impl Viewer {
    fn new(
        path: PathBuf,
        scene_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if !window.is_window_active() {
                this.controls.cancel_drag();
            }
        });
        let mut viewer = Self {
            path,
            scene_index,
            slot: SceneLoadSlot::new(),
            model: None,
            images: ImageCache::default(),
            queue: SceneLoadQueue::new(2.try_into().unwrap(), 2),
            task: None,
            controls: OrbitController::new(Camera::orbit(0.5, 0.3, 5.)).unwrap(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            frame_pending: false,
            selected: None,
            source_camera: None,
            error: None,
            _activation: activation,
        };
        viewer.reload(cx);
        viewer
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let request = self.slot.begin();
        let path = self.path.clone();
        let scene = self.scene_index;
        let images = self.images.clone();
        let queue = self.queue.clone();
        let worker = cx.background_executor().spawn(async move {
            request
                .run(queue.run(move || async move { load(&path, scene, &images).await }))
                .await
        });
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let completion = worker.await;
            let _ = this.update(cx, |this, cx| {
                if publish(&mut this.slot, &mut this.model, completion) {
                    this.frame_pending = true;
                    this.selected = None;
                    this.source_camera = None;
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn frame(&mut self, selected: bool) -> Result<()> {
        let Some(model) = &self.model else {
            return Ok(());
        };
        let bounds = if selected {
            self.selected
                .and_then(|handle| model.evaluated.node(handle))
                .and_then(|node| node.subtree_bounds)
        } else {
            model.evaluated.bounds()
        };
        let Some(bounds) = bounds else {
            return Ok(());
        };
        let rect = self.bounds.get();
        let camera =
            self.controls
                .camera()
                .frame_bounds(bounds, rect.size.width / rect.size.height, 1.2)?;
        self.controls.set_camera(camera)?;
        self.source_camera = None;
        Ok(())
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<gpui::SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px_3()
            .py_2()
            .rounded(px(8.))
            .bg(rgb(0x243951))
            .hover(|s| s.bg(rgb(0x345574)))
            .cursor_pointer()
            .child(label.into())
    }

    fn input(&mut self, result: Result<bool, gpui_3d::OrbitError>, cx: &mut Context<Self>) {
        match result {
            Ok(true) => cx.notify(),
            Ok(false) => {}
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.frame_pending
            && self.bounds.get().size.height > px(0.)
            && self.bounds.get().size.width > px(0.)
        {
            self.frame_pending = false;
            if let Err(error) = self.frame(false) {
                self.error = Some(error.to_string());
            }
        }
        let aspect = self.model.as_ref().and_then(|model| {
            self.source_camera
                .and_then(|node| model.evaluated.node(node))
                .and_then(|node| node.camera)
                .and_then(|camera| camera.aspect_ratio)
        });
        let viewport_size = fitted_size(self.bounds.get().size, aspect);
        let scene = self.model.as_ref().map_or_else(Scene::new, |model| {
            if let Some(camera) = self.source_camera {
                match model.evaluated.scene_from_camera(camera) {
                    Ok(scene) => return scene,
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            model.evaluated.scene(self.controls.camera())
        });
        let mut stage = div()
            .id("stage")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .rounded(px(12.))
            .bg(rgb(0x162337));
        for button in [MouseButton::Right, MouseButton::Middle] {
            stage = stage
                .on_mouse_down(
                    button,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        if this.source_camera.is_some() {
                            return;
                        }
                        let result =
                            this.controls
                                .begin_drag(button, event.position, this.bounds.get());
                        this.input(result, cx);
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(move |this, _, _, _| {
                        this.controls.end_drag(button);
                    }),
                )
                .on_mouse_up_out(
                    button,
                    cx.listener(move |this, _, _, _| {
                        this.controls.end_drag(button);
                    }),
                );
        }
        let bounds = self.bounds.clone();
        let view = cx.entity_id();
        stage = stage
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.source_camera.is_some() {
                    return;
                }
                let result = this.controls.update_drag(
                    event.position,
                    event.pressed_button,
                    this.bounds.get(),
                );
                this.input(result, cx);
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                if !hovered {
                    this.controls.cancel_drag();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                if this.source_camera.is_some() {
                    return;
                }
                let result = this
                    .controls
                    .scroll(f32::from(event.delta.pixel_delta(px(20.)).y));
                this.input(result, cx);
                cx.stop_propagation();
            }))
            .child(
                viewport3d("model", scene)
                    .w(viewport_size.width)
                    .h(viewport_size.height)
                    .on_object_click(cx.listener(|this, hit: &gpui_3d::Hit, _, cx| {
                        this.selected = hit.node;
                        cx.notify();
                    })),
            )
            .child(
                canvas(
                    move |rect, _, cx| {
                        if bounds.replace(rect) != rect {
                            cx.notify(view);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
                .size_full(),
            );
        let details = self
            .model
            .as_ref()
            .map(|m| m.details(self.selected))
            .unwrap_or_else(|| vec!["No model loaded.".into()]);
        let error = self
            .error
            .clone()
            .or_else(|| self.slot.error().map(|e| format!("{e:#}")));
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .bg(rgb(0x0b1422))
            .text_color(rgb(0xeaf2fc))
            .child(div().text_size(px(26.)).child("Model viewer"))
            .child(
                div()
                    .text_sm()
                    .truncate()
                    .text_color(rgb(0x9eb1cb))
                    .child(self.path.to_string_lossy().into_owned()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        self.button("reload", "Reload")
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .child(self.button("cancel", "Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.slot.cancel();
                            this.task = None;
                            cx.notify();
                        },
                    )))
                    .child(self.button("frame", "Frame all").on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Err(error) = this.frame(false) {
                                this.error = Some(error.to_string());
                            }
                            cx.notify();
                        },
                    )))
                    .child(
                        self.button("frame-selection", "Frame selected")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Err(error) = this.frame(true) {
                                    this.error = Some(error.to_string());
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("projection", "Perspective / Ortho")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let mut camera = this.controls.camera();
                                let distance = camera
                                    .eye
                                    .iter()
                                    .zip(camera.target)
                                    .map(|(a, b)| (a - b).powi(2))
                                    .sum::<f32>()
                                    .sqrt();
                                camera.projection = match camera.projection {
                                    Projection::Perspective { vertical_fov } => {
                                        Projection::Orthographic {
                                            vertical_size: 2.
                                                * distance
                                                * (vertical_fov * 0.5).tan(),
                                        }
                                    }
                                    Projection::Orthographic { vertical_size } => {
                                        Projection::Perspective {
                                            vertical_fov: 2.
                                                * (vertical_size / (2. * distance)).atan(),
                                        }
                                    }
                                };
                                if let Err(error) = this.controls.set_camera(camera) {
                                    this.error = Some(error.to_string());
                                }
                                this.source_camera = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "camera",
                            if self.source_camera.is_some() {
                                "Next camera"
                            } else {
                                "Asset camera"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(model) = &this.model {
                                let cameras: Vec<_> = model
                                    .asset()
                                    .nodes()
                                    .iter()
                                    .filter(|n| n.camera_index.is_some())
                                    .filter_map(|n| model.instance.node(n.index))
                                    .collect();
                                this.source_camera = match this.source_camera {
                                    None => cameras.first().copied(),
                                    Some(current) => cameras
                                        .iter()
                                        .position(|h| *h == current)
                                        .and_then(|i| cameras.get(i + 1))
                                        .copied(),
                                };
                                this.controls.cancel_drag();
                                cx.notify();
                            }
                        })),
                    ),
            )
            .child(
                div().flex().flex_1().min_h_0().gap_4().child(stage).child(
                    div()
                        .id("inspector")
                        .w(px(260.))
                        .flex_shrink_0()
                        .h_full()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x8ed6df))
                                .child("SOURCE DETAILS"),
                        )
                        .children(details.into_iter().map(|s| div().text_sm().child(s))),
                ),
            )
            .child(div().text_sm().text_color(rgb(0x9eb1cb)).child(format!(
                "{:?} · {} · Right-drag: orbit · Middle-drag: pan · Scroll: zoom",
                self.slot.status(),
                if self.source_camera.is_some() {
                    "Asset camera"
                } else {
                    "Orbit camera"
                }
            )))
            .when_some(error, |root, error| {
                root.child(
                    div()
                        .id("error")
                        .max_h(px(96.))
                        .overflow_y_scroll()
                        .text_color(rgb(0xf0a59a))
                        .child(error),
                )
            })
            .when(!window.supports_scene3d(), |root| {
                root.child("3D viewports are unavailable on this renderer.")
            })
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().context(USAGE)?;
    if path == "--help" || path == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    let scene = args
        .next()
        .map(|s| s.to_string_lossy().parse::<usize>())
        .transpose()
        .context("invalid scene index")?;
    anyhow::ensure!(args.next().is_none(), USAGE);
    application().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_min_size: Some(size(px(760.), px(520.))),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Viewer::new(path.into(), scene, window, cx)),
        )
        .expect("failed to open model viewer");
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use serde_json::json;

    fn fixture(root: &Path) -> PathBuf {
        let vertices: Vec<u8> = [
            0_f32, 0., 0., 2., 0., 0., 0., 2., 0., 0., 0., 1., 0., 0., 1.,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
        std::fs::write(root.join("mesh.bin"), vertices).unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([90, 120, 180, 255]))
            .save(root.join("color map.png"))
            .unwrap();
        let source = json!({
            "asset":{"version":"2.0"}, "scene":0,
            "buffers":[{"uri":"mesh.bin","byteLength":60}],
            "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}, {"buffer":0,"byteOffset":36,"byteLength":24}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[2,2,0]},
                {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
            "images":[{"uri":"color%20map.png"}], "textures":[{"source":0}],
            "materials":[{"name":"Coating","doubleSided":true,"pbrMetallicRoughness":{"metallicFactor":0.2,"baseColorTexture":{"index":0}}}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],
            "nodes":[{"translation":[3,2,0],"children":[1]},{"name":"Panel","mesh":0}],
            "scenes":[{"nodes":[0]}]
        });
        let path = root.join("scene.gltf");
        std::fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
        path
    }

    #[test]
    fn background_file_loading_publishes_bound_geometry_and_material_metadata() {
        let temporary = tempfile::tempdir().unwrap();
        let path = fixture(temporary.path());
        let mut slot = SceneLoadSlot::new();
        let request = slot.begin();
        let completion = std::thread::spawn(move || {
            let queue = SceneLoadQueue::new(1.try_into().unwrap(), 1);
            block_on(
                request
                    .run(queue.run(|| async { load(&path, None, &ImageCache::default()).await })),
            )
        })
        .join()
        .unwrap();
        let mut model = None;
        assert!(publish(&mut slot, &mut model, completion));
        let model = model.unwrap();
        let bounds = model.evaluated.bounds().unwrap();
        assert_eq!(bounds.min(), [3., 2., 0.]);
        assert_eq!(bounds.max(), [5., 4., 0.]);
        let viewport = Bounds::new(gpui::point(px(23.), px(17.)), size(px(640.), px(360.)));
        let camera = Camera::orbit(0., 0., 5.)
            .frame_bounds(bounds, 640. / 360., 1.2)
            .unwrap();
        let point = camera
            .world_to_screen(viewport, [3.5, 2.5, 0.])
            .unwrap()
            .unwrap();
        let hit = model
            .evaluated
            .scene(camera)
            .pick(viewport, point.position)
            .unwrap();
        let primitive = model.instance.source_primitive(hit.node.unwrap()).unwrap();
        assert_eq!(
            (
                primitive.node_index,
                primitive.mesh_index,
                primitive.material_index
            ),
            (1, 0, Some(0))
        );
        let details = model.details(hit.node).join("\n");
        assert!(
            details.contains("Panel")
                && details.contains("Coating")
                && details.contains("Metallic 0.20")
        );
    }

    #[test]
    fn failed_and_superseded_reloads_preserve_the_displayed_model() {
        let temporary = tempfile::tempdir().unwrap();
        let path = fixture(temporary.path());
        let images = ImageCache::default();
        let mut slot = SceneLoadSlot::new();
        let mut model = None;
        let ready = block_on(slot.begin().run(load(&path, None, &images)));
        assert!(publish(&mut slot, &mut model, ready));
        let root = model.as_ref().unwrap().instance.root();
        let old = block_on(slot.begin().run(load(&path, None, &images)));
        let current = slot.begin();
        assert!(!publish(&mut slot, &mut model, old));
        assert_eq!(model.as_ref().unwrap().instance.root(), root);
        std::fs::write(&path, b"invalid json").unwrap();
        let failed = block_on(current.run(load(&path, None, &images)));
        assert!(!publish(&mut slot, &mut model, failed));
        assert_eq!(slot.status(), SceneLoadStatus::Failed);
        assert!(slot.error().is_some());
        assert_eq!(model.as_ref().unwrap().instance.root(), root);
        assert_eq!(slot.asset().unwrap().primitives().len(), 1);
    }

    #[test]
    fn fixed_camera_aspect_fits_portrait_and_landscape_stages_without_stretching() {
        for available in [size(px(900.), px(300.)), size(px(300.), px(900.))] {
            for aspect in [0.5, 2.] {
                let fitted = fitted_size(available, Some(aspect));
                assert!(fitted.width <= available.width && fitted.height <= available.height);
                assert_eq!(fitted.width / fitted.height, aspect);
                assert!(fitted.width == available.width || fitted.height == available.height);
            }
            assert_eq!(fitted_size(available, None), available);
        }
    }
}
