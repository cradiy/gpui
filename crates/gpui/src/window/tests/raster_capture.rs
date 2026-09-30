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

#[crate::test]
fn raster_capture_limits_allocation_at_large_window_sizes(cx: &mut TestAppContext) {
    // Test windows use a 2x display density. The last case already exceeds the
    // cap at native density and must not be supersampled or downsampled.
    for viewport in [
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
