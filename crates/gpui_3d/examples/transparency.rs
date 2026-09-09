use gpui::{
    App, Bounds, Context, Render, RenderImage, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, rgba, size,
};
use gpui_3d::{AlphaMode, Camera, Material, Mesh, Object, Scene, viewport3d};
use gpui_platform::application;
use std::sync::Arc;

struct Transparency {
    image: Arc<RenderImage>,
    checker: Arc<RenderImage>,
    opacity: f32,
    reversed: bool,
}

impl Transparency {
    fn new() -> Self {
        let image = image::RgbaImage::from_fn(128, 128, |x, y| {
            let alpha = if (48..80).contains(&x) && (48..80).contains(&y) {
                0
            } else {
                (x * 255 / 127) as u8
            };
            image::Rgba([255, 255, 255, alpha])
        });
        let checker = image::RgbaImage::from_fn(128, 128, |x, y| {
            let value = if (x / 16 + y / 16) % 2 == 0 { 46 } else { 80 };
            image::Rgba([value, value, value, 255])
        });
        Self {
            image: Arc::new(RenderImage::new(vec![image::Frame::new(image)])),
            checker: Arc::new(RenderImage::new(vec![image::Frame::new(checker)])),
            opacity: 0.85,
            reversed: false,
        }
    }

    fn scene(&self, mode: AlphaMode) -> Scene {
        let mut cyan = rgba(0x65e0f5ff);
        cyan.a = self.opacity;
        let mut coral = rgba(0xff9580ff);
        coral.a = self.opacity;
        let layer = |tint| {
            Material::image(self.image.clone())
                .tint(tint)
                .unlit(true)
                .alpha_mode(mode)
        };
        let mut objects = vec![
            Object::new(Mesh::plane(), layer(cyan))
                .position([0.22, 0.2, 0.35])
                .scale([1.5, 1.5, 1.]),
            Object::new(
                Mesh::plane(),
                Material::image(self.checker.clone()).unlit(true),
            )
            .position([0., 0., -0.4])
            .scale([2.2, 2.2, 1.]),
            Object::new(Mesh::plane(), layer(coral))
                .position([-0.22, -0.2, 0.])
                .scale([1.5, 1.5, 1.]),
        ];
        if self.reversed {
            objects.reverse();
        }
        objects.into_iter().fold(
            Scene::new().camera(Camera::orbit(0., 0., 3.8)),
            |scene, object| scene.object(object),
        )
    }
}

impl Render for Transparency {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x0c1421))
            .text_color(rgb(0xe8f0fa))
            .child(div().text_size(px(30.)).child("Layers of light"))
            .child(
                div()
                    .text_color(rgb(0x95aac5))
                    .child("One alpha gradient · Two overlapping planes · Checkerboard behind"),
            )
            .child(
                div().flex().gap_3().children(
                    [
                        ("less", "Opacity −"),
                        ("more", "Opacity +"),
                        ("reverse", "Reverse submission"),
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
                                    "less" => this.opacity = (this.opacity - 0.1).max(0.),
                                    "more" => this.opacity = (this.opacity + 0.1).min(1.),
                                    _ => this.reversed = !this.reversed,
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                div().flex().gap_4().flex_1().min_h_0().children(
                    [
                        ("Opaque", AlphaMode::Opaque, "Alpha ignored"),
                        ("Mask", AlphaMode::Mask, "Cutoff 0.50"),
                        ("Blend", AlphaMode::Blend, "Continuous transparency"),
                    ]
                    .into_iter()
                    .map(|(label, mode, description)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .justify_center()
                            .child(div().text_size(px(22.)).child(label))
                            .child(
                                div()
                                    .w_full()
                                    .aspect_ratio(1.)
                                    .rounded(px(16.))
                                    .overflow_hidden()
                                    .bg(rgb(0x142237))
                                    .child(viewport3d(label, self.scene(mode)).size_full()),
                            )
                            .child(div().text_color(rgb(0x95aac5)).child(description))
                    }),
                ),
            )
            .child(format!(
                "Opacity {:.0}% · Submission {}",
                self.opacity * 100.,
                if self.reversed { "reversed" } else { "forward" }
            ))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Transparency::new()),
        )
        .expect("failed to open transparency example");
    });
}
