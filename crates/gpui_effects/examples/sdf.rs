use gpui::{
    App, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, Render,
    Window, WindowBounds, WindowOptions, canvas, div, point, prelude::*, px, rgb, size,
};
use gpui_effects::{SdfOptions, SdfScene, SdfShape, SdfTransform};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc};

const POSITIONS: [[f32; 2]; 3] = [[0.43, 0.44], [0.57, 0.44], [0.5, 0.64]];
const COLORS: [u32; 3] = [0x61dce6, 0xa38aef, 0xf2ad8a];
const MODES: [&str; 3] = ["Fusion", "Cutout", "Overlap"];
struct SdfPreview {
    scenes: [SdfScene; 3],
    guide: SdfScene,
    positions: [[f32; 2]; 3],
    bounds: Rc<Cell<Bounds<Pixels>>>,
    dragging: Option<(usize, Point<Pixels>)>,
    mode: usize,
    smooth: bool,
    outline: bool,
    glow: bool,
    rotation: f32,
}
impl SdfPreview {
    fn new() -> Self {
        let origin = point(px(0.), px(0.));
        let circle = SdfShape::circle(origin, px(80.), rgb(COLORS[0]));
        let rect =
            SdfShape::rounded_rect(origin, size(px(164.), px(144.)), px(32.), rgb(COLORS[1]));
        let capsule = SdfShape::capsule(origin, px(100.), px(40.), rgb(COLORS[2]));
        let base = circle.smooth_union(rect);
        let scenes = [
            base.clone().smooth_union(capsule.clone()),
            base.clone().smooth_subtract(capsule.clone()),
            base.smooth_intersect(capsule.clone()),
        ]
        .map(|shape| SdfScene::new(shape).expect("invalid shape composition"));
        let mut guide_color = rgb(COLORS[2]);
        guide_color.a = 0.35;
        let mut guide = SdfScene::new(SdfShape::capsule(origin, px(100.), px(40.), guide_color))
            .expect("invalid capsule");
        guide.set_options(SdfOptions {
            fill_opacity: 0.,
            stroke_width: px(1.),
            ..Default::default()
        });
        Self {
            scenes,
            guide,
            positions: POSITIONS,
            bounds: Rc::new(Cell::new(Bounds::default())),
            dragging: None,
            mode: 0,
            smooth: true,
            outline: false,
            glow: true,
            rotation: 0.,
        }
    }
    fn center(&self, index: usize) -> Point<Pixels> {
        let size = self.bounds.get().size;
        point(
            size.width * self.positions[index][0],
            size.height * self.positions[index][1],
        )
    }
    fn begin_drag(&mut self, position: Point<Pixels>) {
        let local = position - self.bounds.get().origin;
        self.dragging = (0..3).rev().find_map(|i| {
            let relative = local - self.center(i);
            let mut x = f32::from(relative.x);
            let mut y = f32::from(relative.y);
            if i == 2 {
                let (s, c) = self.rotation.sin_cos();
                (x, y) = (c * x + s * y, -s * x + c * y);
            }
            let distance = match i {
                0 => x.hypot(y) - 80.,
                1 => {
                    let qx = x.abs() - 50.;
                    let qy = y.abs() - 40.;
                    qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - 32.
                }
                _ => (x - x.clamp(-50., 50.)).hypot(y) - 40.,
            };
            (distance <= 6.).then_some((i, relative))
        });
    }
    fn drag(&mut self, position: Point<Pixels>) {
        if let Some((index, offset)) = self.dragging {
            let local = position - self.bounds.get().origin - offset;
            let extent = self.bounds.get().size;
            self.positions[index] = [
                (f32::from(local.x) / f32::from(extent.width).max(1.)).clamp(0.08, 0.92),
                (f32::from(local.y) / f32::from(extent.height).max(1.)).clamp(0.12, 0.88),
            ];
        }
    }
}
fn button(id: &'static str, text: &'static str, selected: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_4()
        .py_2()
        .rounded_full()
        .cursor_pointer()
        .bg(rgb(if selected { 0x34415c } else { 0x192338 }))
        .hover(|style| style.bg(rgb(0x3b4964)))
        .child(text)
}
impl Render for SdfPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut scene = self.scenes[self.mode].clone();
        scene.set_options(SdfOptions {
            smoothing: px(if self.smooth { 48. } else { 0. }),
            fill_opacity: if self.outline { 0. } else { 1. },
            stroke_width: px(if self.outline { 2. } else { 0. }),
            outer_glow: px(if self.glow { 22. } else { 0. }),
            outer_glow_opacity: 0.22,
            inner_glow: px(if self.glow { 14. } else { 0. }),
            inner_glow_intensity: 0.4,
        });
        let positions = self.positions;
        let bounds = self.bounds.clone();
        let rotation = self.rotation;
        let mut guide = self.guide.clone();
        let show_guide = self.mode != 0;
        let surface = canvas(
            move |region, _, _| bounds.set(region),
            move |region, _, window, _| {
                for (i, position) in positions.iter().enumerate() {
                    scene.set_transform(
                        i,
                        SdfTransform {
                            center: point(
                                region.size.width * position[0],
                                region.size.height * position[1],
                            ),
                            rotation: if i == 2 { rotation } else { 0. },
                            ..Default::default()
                        },
                    );
                }
                let _ = scene.paint(region, window);
                if show_guide {
                    guide.set_transform(0, scene.transform(2).unwrap());
                    let _ = guide.paint(region, window);
                }
            },
        )
        .size_full();
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x090e19))
            .text_color(rgb(0xe8edfa))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Soft geometry"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8996af))
                                    .child("Drag a shape. Blend, carve, and intersect."),
                            ),
                    )
                    .child(div().flex().gap_2().children(MODES.iter().enumerate().map(
                        |(i, &label)| {
                            button(label, label, self.mode == i).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.mode = i;
                                    this.dragging = None;
                                    cx.notify();
                                },
                            ))
                        },
                    ))),
            )
            .child(
                div()
                    .id("sdf-surface")
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(24.))
                    .overflow_hidden()
                    .bg(rgb(0x11192a))
                    .cursor_crosshair()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.begin_drag(event.position);
                            cx.notify();
                        }),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left)
                            && this.dragging.is_some()
                        {
                            this.drag(event.position);
                            cx.notify();
                        } else {
                            this.dragging = None;
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, _| this.dragging = None),
                    )
                    .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                        if !*hovered {
                            this.dragging = None;
                        }
                    }))
                    .child(surface),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .children(
                                COLORS
                                    .map(|color| div().size(px(10.)).rounded_full().bg(rgb(color))),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8996af))
                                    .child("Circle · Rounded rectangle · Capsule"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                button(
                                    "blend",
                                    if self.smooth {
                                        "Blend · Soft"
                                    } else {
                                        "Blend · Hard"
                                    },
                                    self.smooth,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.smooth = !this.smooth;
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(button("outline", "Outline", self.outline).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.outline = !this.outline;
                                    cx.notify();
                                }),
                            ))
                            .child(button("glow", "Glow", self.glow).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.glow = !this.glow;
                                    cx.notify();
                                },
                            )))
                            .child(button("rotate", "Rotate", false).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.rotation += std::f32::consts::FRAC_PI_4;
                                    cx.notify();
                                },
                            )))
                            .child(button("reset", "Reset", false).on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.positions = POSITIONS;
                                    this.rotation = 0.;
                                    this.dragging = None;
                                    cx.notify();
                                },
                            ))),
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
                    size(px(1100.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| SdfPreview::new()),
        )
        .expect("failed to open SDF example");
    });
}
