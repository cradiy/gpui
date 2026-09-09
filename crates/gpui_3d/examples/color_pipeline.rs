use gpui::{
    App, Bounds, Context, Render, RenderImage, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, ColorOutput, Light, Material, Mesh, Object, Scene, TextureColorSpace, ToneMapping,
    viewport3d,
};
use gpui_platform::application;
use std::sync::Arc;

struct ColorPipeline {
    image: Arc<RenderImage>,
    cube: Mesh,
    exposure: f32,
    intensity: f32,
    image_space: TextureColorSpace,
}

impl ColorPipeline {
    fn new() -> Self {
        let pixels = image::RgbaImage::from_fn(64, 64, |x, _| {
            let value = 48 + (x * 192 / 63) as u8;
            image::Rgba([value, value, value, 255])
        });
        Self {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
            cube: Mesh::cube(),
            exposure: 0.,
            intensity: 4.,
            image_space: TextureColorSpace::Srgb,
        }
    }

    fn scene(&self, tone_mapping: ToneMapping) -> Scene {
        let mut scene = Scene::new()
            .camera(Camera::orbit(0.2, 0.25, 6.5))
            .color_output(ColorOutput {
                exposure: self.exposure,
                tone_mapping,
            })
            .light(Light {
                direction: [-0.4, 0.7, 1.],
                color: rgb(0xffffff),
                intensity: self.intensity,
                ambient: 0.12,
            });
        for (index, color) in [0xf0a47f, 0x88d5e3, 0xc5a5ee].into_iter().enumerate() {
            scene = scene.object(
                Object::new(
                    self.cube.clone(),
                    Material::image(self.image.clone())
                        .image_color_space(self.image_space)
                        .tint(rgb(color)),
                )
                .position([(index as f32 - 1.) * 1.35, 0., 0.])
                .rotation([0., -0.35, 0.]),
            );
        }
        scene
    }
}

impl Render for ColorPipeline {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0c1421))
            .text_color(rgb(0xe8f0fa))
            .child(div().text_size(px(30.)).child("Light and exposure"))
            .child(div().text_color(rgb(0x95aac5)).child(
                "The same scene, two highlight mappings. Lower exposure to recover bright detail.",
            ))
            .child(
                div().flex().flex_wrap().gap_3().children(
                    [
                        ("darker", "Exposure −".to_owned()),
                        ("brighter", "Exposure +".to_owned()),
                        ("less", "Light −".to_owned()),
                        ("more", "Light +".to_owned()),
                        ("space", format!("Image: {:?}", self.image_space)),
                        ("reset", "Reset".to_owned()),
                    ]
                    .into_iter()
                    .map(|(id, label)| {
                        div()
                            .id(id)
                            .px_4()
                            .py_2()
                            .rounded(px(10.))
                            .bg(rgb(0x263d56))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(0x365974)))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match id {
                                    "darker" => this.exposure = (this.exposure - 0.5).max(-8.),
                                    "brighter" => this.exposure = (this.exposure + 0.5).min(8.),
                                    "less" => this.intensity = (this.intensity - 0.5).max(0.),
                                    "more" => this.intensity = (this.intensity + 0.5).min(16.),
                                    "space" => {
                                        this.image_space =
                                            if this.image_space == TextureColorSpace::Srgb {
                                                TextureColorSpace::Linear
                                            } else {
                                                TextureColorSpace::Srgb
                                            }
                                    }
                                    _ => {
                                        this.exposure = 0.;
                                        this.intensity = 4.;
                                        this.image_space = TextureColorSpace::Srgb;
                                    }
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div().flex().flex_1().min_h_0().gap_4().children(
                    [
                        ("clamp", "Clipped highlights", ToneMapping::None),
                        (
                            "reinhard",
                            "Compressed highlights · Reinhard",
                            ToneMapping::Reinhard,
                        ),
                    ]
                    .into_iter()
                    .map(|(id, label, mapping)| {
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .child(div().text_size(px(20.)).child(label))
                            .child(
                                viewport3d(id, self.scene(mapping))
                                    .w_full()
                                    .flex_1()
                                    .min_h_0(),
                            )
                    }),
                ),
            )
            .child(div().text_color(rgb(0xa8bdd6)).child(format!(
                "Exposure {:+.1} EV · Light {:.1}×",
                self.exposure, self.intensity
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child("GPUI reference colors")
                    .children(
                        [0xf0a47f, 0x88d5e3, 0xc5a5ee].map(|color| {
                            div().w(px(64.)).h(px(24.)).rounded(px(6.)).bg(rgb(color))
                        }),
                    ),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(720.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ColorPipeline::new()),
        )
        .expect("failed to open color pipeline example");
    });
}
