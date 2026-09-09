use gpui::{
    App, Bounds, Context, Render, RenderImage, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use gpui_3d::{
    Camera, Material, Mesh, Object, Projection, Scene, TextureAddressMode, TextureFilter,
    TextureSampling, UvTransform, viewport3d,
};
use gpui_platform::application;
use std::sync::Arc;

struct Sampling {
    image: Arc<RenderImage>,
    filter: TextureFilter,
    scale: f32,
    rotation: f32,
    offset: f32,
    selected: String,
}

impl Sampling {
    fn new() -> Self {
        let pixels = image::RgbaImage::from_fn(16, 16, |x, y| {
            let colors = [
                [236, 157, 124, 255],
                [121, 208, 220, 255],
                [188, 157, 235, 255],
                [246, 218, 146, 255],
            ];
            let mut color = colors[((x / 4 + y / 4) % 4) as usize];
            if (6..10).contains(&x) && (5..11).contains(&y) {
                color[3] = 0;
            }
            image::Rgba(color)
        });
        Self {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
            filter: TextureFilter::Linear,
            scale: 2.,
            rotation: 0.,
            offset: -0.25,
            selected: "Click a tile or its transparent opening".into(),
        }
    }

    fn scene(&self, mode: TextureAddressMode) -> Scene {
        let sampling = TextureSampling {
            transform: UvTransform::from_scale_rotation_translation(
                [self.scale; 2],
                self.rotation,
                [self.offset; 2],
            )
            .unwrap(),
            address_u: mode,
            address_v: mode,
            filter: self.filter,
        };
        Scene::new()
            .camera(Camera {
                projection: Projection::Orthographic { vertical_size: 1.2 },
                ..Default::default()
            })
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0x25344d)).unlit(true))
                    .id("Background")
                    .position([0., 0., -0.1]),
            )
            .object(
                Object::new(
                    Mesh::plane(),
                    Material::image(self.image.clone())
                        .unlit(true)
                        .image_sampling(sampling),
                )
                .id("Image"),
            )
    }
}

impl Render for Sampling {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let controls = [
            ("filter", format!("Filter: {:?}", self.filter)),
            ("smaller", "Scale −".into()),
            ("larger", "Scale +".into()),
            ("rotate", "Rotate".into()),
            ("shift", "Offset".into()),
            ("reset", "Reset".into()),
        ];
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_5()
            .bg(rgb(0x0c1421))
            .text_color(rgb(0xe8f0fa))
            .child(div().text_size(px(30.)).child("Texture coordinates"))
            .child(
                div()
                    .text_color(rgb(0x95aac5))
                    .child("One image · Three addressing modes · Alpha-aware picking"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .children(controls.into_iter().map(|(id, label)| {
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
                                    "filter" => {
                                        this.filter = if this.filter == TextureFilter::Nearest {
                                            TextureFilter::Linear
                                        } else {
                                            TextureFilter::Nearest
                                        }
                                    }
                                    "smaller" => this.scale = (this.scale - 0.25).max(0.25),
                                    "larger" => this.scale = (this.scale + 0.25).min(8.),
                                    "rotate" => this.rotation += std::f32::consts::FRAC_PI_8,
                                    "shift" => this.offset += 0.125,
                                    _ => *this = Self::new(),
                                }
                                cx.notify();
                            }))
                    })),
            )
            .child(
                div().flex().flex_1().min_h_0().gap_4().children(
                    [
                        ("Clamp", TextureAddressMode::Clamp),
                        ("Repeat", TextureAddressMode::Repeat),
                        ("Mirror", TextureAddressMode::Mirror),
                    ]
                    .into_iter()
                    .map(|(label, mode)| {
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .child(div().text_size(px(20.)).child(label))
                            .child(
                                viewport3d(label, self.scene(mode))
                                    .w_full()
                                    .flex_1()
                                    .min_h_0()
                                    .on_object_click(cx.listener(
                                        move |this, hit: &gpui_3d::Hit, _, cx| {
                                            this.selected = format!(
                                                "{label} · {:?} · mesh UV ({:.2}, {:.2})",
                                                hit.object_id, hit.uv[0], hit.uv[1]
                                            );
                                            cx.notify();
                                        },
                                    )),
                            )
                    }),
                ),
            )
            .child(div().text_color(rgb(0xa8bdd6)).child(format!(
                "Scale {:.2} · Rotation {:.0}° · Offset {:.3}",
                self.scale,
                self.rotation.to_degrees(),
                self.offset
            )))
            .child(self.selected.clone())
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(720.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Sampling::new()),
        )
        .expect("failed to open texture sampling example");
    });
}
