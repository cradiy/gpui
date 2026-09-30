use crate::{
    AppContext, Bounds, Context, DevicePixels, EffectShader, Entity, Font, FontId, FontMetrics,
    FontRun, GlyphId, IntoElement, LineLayout, MouseButton, MouseDownEvent, NoopTextSystem, Pixels,
    PlatformInput, PlatformTextSystem, Point, Render, RenderGlyphParams, ScaledPixels, Size,
    TestAppContext, TextRenderingMode, TextSystem, Window, WindowTextSystem, canvas, div, fill,
    point, prelude::*, px, rgb, size,
};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

// A density-aware glyph backend makes atlas allocation observable without OS fonts.
struct RasterText;
impl PlatformTextSystem for RasterText {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> anyhow::Result<()> {
        NoopTextSystem.add_fonts(fonts)
    }
    fn all_font_names(&self) -> Vec<String> {
        NoopTextSystem.all_font_names()
    }
    fn font_id(&self, font: &Font) -> anyhow::Result<FontId> {
        NoopTextSystem.font_id(font)
    }
    fn font_metrics(&self, id: FontId) -> FontMetrics {
        NoopTextSystem.font_metrics(id)
    }
    fn typographic_bounds(&self, font: FontId, glyph: GlyphId) -> anyhow::Result<Bounds<f32>> {
        NoopTextSystem.typographic_bounds(font, glyph)
    }
    fn advance(&self, font: FontId, glyph: GlyphId) -> anyhow::Result<Size<f32>> {
        NoopTextSystem.advance(font, glyph)
    }
    fn glyph_for_char(&self, font: FontId, ch: char) -> Option<GlyphId> {
        NoopTextSystem.glyph_for_char(font, ch)
    }
    fn layout_line(&self, text: &str, size: Pixels, runs: &[FontRun]) -> LineLayout {
        NoopTextSystem.layout_line(text, size, runs)
    }
    fn recommended_rendering_mode(&self, _: FontId, _: Pixels) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
    fn glyph_raster_bounds(
        &self,
        params: &RenderGlyphParams,
    ) -> anyhow::Result<Bounds<DevicePixels>> {
        Ok(Bounds::new(
            Default::default(),
            size(
                DevicePixels((4. * params.scale_factor) as i32),
                DevicePixels((6. * params.scale_factor) as i32),
            ),
        ))
    }
    fn rasterize_glyph(
        &self,
        _: &RenderGlyphParams,
        bounds: Bounds<DevicePixels>,
    ) -> anyhow::Result<(Size<DevicePixels>, Vec<u8>)> {
        Ok((
            bounds.size,
            vec![255; (bounds.size.width.0 * bounds.size.height.0) as usize],
        ))
    }
}

struct Content {
    renders: Rc<Cell<usize>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    clicks: Rc<RefCell<Vec<Point<Pixels>>>>,
}
impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let bounds = self.bounds.clone();
        let clicks = self.clicks.clone();
        div()
            .size_full()
            .on_mouse_down(MouseButton::Left, move |e, _, _| {
                clicks.borrow_mut().push(e.position)
            })
            .child(
                canvas(
                    move |actual, _, _| bounds.set(actual),
                    |bounds, _, window, _| {
                        window.paint_quad(fill(bounds, rgb(0xff0000)));
                        window
                            .paint_glyph(
                                bounds.origin,
                                FontId(0),
                                GlyphId(1),
                                px(16.),
                                rgb(0xffffff).into(),
                            )
                            .unwrap();
                    },
                )
                .size_full(),
            )
    }
}

struct Root {
    content: Entity<Content>,
    density: f32,
}

struct RegionContent {
    full_window: bool,
    renders: Rc<Cell<usize>>,
}

fn identity_shader() -> EffectShader {
    EffectShader::wgsl_image(
        "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
    )
}

impl Render for RegionContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let full = self.full_window;
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                if full {
                    // An ordinary nested effect requires full-window source pixels.
                    window.with_subtree_effect(
                        bounds,
                        identity_shader(),
                        Default::default(),
                        0.,
                        1.,
                        |window| {
                            window.paint_quad(fill(bounds, rgb(0xff0000)));
                        },
                    );
                } else {
                    window.paint_quad(fill(bounds, rgb(0xff0000)));
                }
            },
        )
        .size_full()
    }
}

struct RegionRoot {
    content: Entity<RegionContent>,
    bounds: Bounds<Pixels>,
}

impl Render for RegionRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let content = self.content.clone();
        let bounds = self.bounds;
        canvas(
            move |_, window, cx| {
                window.prepaint_subtree_effect(|window| {
                    window.with_subtree_raster_scale_in(bounds, 4., |window| {
                        let mut child = content
                            .cached(
                                div()
                                    .w(bounds.size.width)
                                    .h(bounds.size.height)
                                    .style()
                                    .clone(),
                            )
                            .into_any_element();
                        child.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                        child
                    })
                })
            },
            move |_, mut child, window, cx| {
                window.with_subtree_effect(
                    bounds,
                    identity_shader(),
                    Default::default(),
                    0.,
                    1.,
                    |window| {
                        window.with_subtree_raster_scale_in(bounds, 4., |window| {
                            child.paint(window, cx)
                        });
                    },
                );
            },
        )
        .size_full()
    }
}

#[crate::test]
fn raster_region_budget_preserves_small_captures_and_recovers_from_full_window_effects(
    cx: &mut TestAppContext,
) {
    let renders = Rc::new(Cell::new(0));
    let handle = cx.open_window(size(px(2000.), px(1400.)), {
        let renders = renders.clone();
        move |_, cx| RegionRoot {
            content: cx.new(|_| RegionContent {
                full_window: false,
                renders,
            }),
            bounds: Bounds::new(point(px(30.4), px(40.4)), size(px(100.), px(80.))),
        }
    });
    cx.set_subtree_effects_supported(handle.into(), true);
    let mut previous_renders = 0;
    for (full, changed) in [
        (false, false),
        (true, true),
        (true, false),
        (false, true),
        (false, false),
    ] {
        if changed {
            handle
                .update(cx, |root, _, cx| {
                    root.content.update(cx, |content, cx| {
                        content.full_window = full;
                        cx.notify();
                    })
                })
                .unwrap();
        }
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let scale = window.rendered_frame.scene.subtree_layers[0]
                .scene
                .raster_scale
                .unwrap();
            if full {
                assert!(scale > 1. && scale < 2.);
                let width = (4000. * scale).ceil() as u64;
                let height = (2800. * scale).ceil() as u64;
                assert!(width * height <= 16_777_216);
                assert!(!window.raster_budget_retrying);
            } else {
                // Removing a viewport-sized effect schedules a density upgrade.
                window.draw(cx).clear();
                let source = &window.rendered_frame.scene.subtree_layers[0].scene;
                assert_eq!(source.raster_scale, Some(4.));
                assert_eq!(
                    source.quads[0].bounds.size,
                    size(ScaledPixels(800.), ScaledPixels(640.))
                );
            }
        })
        .unwrap();
        if !changed && previous_renders > 0 {
            assert_eq!(renders.get(), previous_renders);
        }
        previous_renders = renders.get();
    }
}

#[crate::test]
fn raster_region_budget_limits_fractional_clipped_and_large_sources(cx: &mut TestAppContext) {
    let viewport = size(px(5000.), px(3000.));
    for bounds in [
        Bounds::new(point(px(30.4), px(40.4)), size(px(100.), px(80.))),
        Bounds::new(point(px(30.4), px(40.4)), size(px(1700.3), px(1300.7))),
        Bounds::new(point(px(-900.4), px(40.4)), size(px(1000.), px(100.))),
        Bounds::new(point(px(4900.4), px(40.4)), size(px(1000.), px(100.))),
    ] {
        let handle = cx.open_window(viewport, move |_, cx| RegionRoot {
            content: cx.new(|_| RegionContent {
                full_window: false,
                renders: Default::default(),
            }),
            bounds,
        });
        cx.set_subtree_effects_supported(handle.into(), true);
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let layer = &window.rendered_frame.scene.subtree_layers[0];
            let scale = layer.scene.raster_scale.unwrap();
            // The base window already exceeds the full-window pixel budget.
            assert!(scale > 1.);
            let capture = layer.composite.bounds;
            let width = (capture.right().0 * scale)
                .ceil()
                .min((10000. * scale).ceil())
                - (capture.left().0 * scale).floor().max(0.);
            let height = (capture.bottom().0 * scale)
                .ceil()
                .min((6000. * scale).ceil())
                - (capture.top().0 * scale).floor().max(0.);
            assert!(width <= 8192. && height <= 8192.);
            assert!(f64::from(width) * f64::from(height) <= 16_777_216.);
            if bounds.size.height < px(200.) {
                assert_eq!(scale, 4.);
            }
        })
        .unwrap();
    }
}

#[crate::test]
fn raster_capture_limits_allocation_at_large_window_sizes(cx: &mut TestAppContext) {
    // Test windows use a 2x display density. The last case already exceeds the
    // cap at native density and must not be supersampled or downsampled.
    for viewport in [
        // 1027 × 1170 physical pixels exercises a rounded allocation limit.
        size(px(513.5), px(585.)),
        size(px(1301.), px(733.)),
        size(px(3000.), px(300.)),
        size(px(5000.), px(3000.)),
    ] {
        let handle = cx.open_window(viewport, |_, cx| Root {
            content: cx.new(|_| Content {
                renders: Default::default(),
                bounds: Default::default(),
                clicks: Default::default(),
            }),
            density: 20.,
        });
        cx.set_subtree_effects_supported(handle.into(), true);
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let scale = window.rendered_frame.scene.subtree_layers[0]
                .scene
                .raster_scale
                .unwrap_or(1.);
            if viewport.width == px(5000.) {
                assert_eq!(scale, 1.);
            } else {
                assert!(scale > 1. && scale <= 4.);
                let width = (viewport.width.0 * 2. * scale).ceil() as u64;
                let height = (viewport.height.0 * 2. * scale).ceil() as u64;
                assert!(width <= 8192 && height <= 8192);
                assert!(width * height <= 16_777_216);
            }
        })
        .unwrap();
    }
}
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let content = self.content.clone();
        let density = self.density;
        canvas(move |_, window, cx| {
            window.prepaint_subtree_effect(|window| window.with_subtree_raster_scale(density, |window| {
                let mut child = content.cached(div().w(px(100.)).h(px(80.)).style().clone()).into_any_element();
                child.prepaint_as_root(point(px(10.4), px(20.4)), size(px(100.), px(80.)).into(), window, cx);
                child
            }))
        }, move |bounds, mut child, window, cx| {
            window.with_subtree_effect(bounds, EffectShader::wgsl_image("fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }"), Default::default(), 0., 1., |window| {
                window.with_subtree_raster_scale(density, |window| child.paint(window, cx));
            });
        }).size_full()
    }
}

#[crate::test]
fn raster_capture_updates_glyph_atlas_and_cached_paint_without_moving_layout_or_input(
    cx: &mut TestAppContext,
) {
    let renders = Rc::new(Cell::new(0));
    let bounds = Rc::new(Cell::new(Bounds::default()));
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let handle = cx.open_window(size(px(400.), px(300.)), {
        let (renders, bounds, clicks) = (renders.clone(), bounds.clone(), clicks.clone());
        move |window, cx| {
            window.mouse_position = point(px(30.), px(40.));
            window.text_system = Arc::new(WindowTextSystem::new(Arc::new(TextSystem::new(
                Arc::new(RasterText),
            ))));
            Root {
                content: cx.new(|_| Content {
                    renders,
                    bounds,
                    clicks,
                }),
                density: 1.,
            }
        }
    });
    cx.set_subtree_effects_supported(handle.into(), true);
    cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    renders.set(0);
    let mut logical_bounds = None;
    let mut previous_tile = None;
    for (density, count) in [(1., 0), (1., 0), (2., 1), (2., 1), (1., 2), (2., 3)] {
        handle
            .update(cx, |root, _, cx| {
                root.density = density;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let source = &window.rendered_frame.scene.subtree_layers[0].scene;
            assert_eq!(source.raster_scale, Some(density));
            assert_eq!(
                source.quads[0].bounds.size,
                size(ScaledPixels(200. * density), ScaledPixels(160. * density))
            );
            let glyph = &source.monochrome_sprites[0];
            assert_eq!(
                glyph.tile.bounds.size,
                size(
                    DevicePixels((8. * density) as i32),
                    DevicePixels((12. * density) as i32)
                )
            );
            if let Some((previous_density, tile)) = previous_tile {
                assert_eq!(tile == glyph.tile, previous_density == density);
            }
            previous_tile = Some((density, glyph.tile));
            assert_eq!(window.scale_factor(), 2.);
            assert_eq!(window.raster_scale_factor(), 2.);
        })
        .unwrap();
        assert_eq!(*logical_bounds.get_or_insert(bounds.get()), bounds.get());
        assert_eq!(
            renders.get(),
            count,
            "density={density}, expected render={count}"
        );
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: point(px(30.), px(40.)),
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
    })
    .unwrap();
    assert_eq!(&*clicks.borrow(), &[point(px(30.), px(40.))]);
}
