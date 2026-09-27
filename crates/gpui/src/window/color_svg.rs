use super::Window;
use crate::{App, RenderColorSvgParams, Task};
use collections::FxHashMap;
use scheduler::Instant;
use std::{borrow::Cow, time::Duration};

const MAX_CONCURRENT_RASTERS: usize = 2;
const MAX_SYNC_PIXELS: i64 = 256 * 256;

#[derive(Default)]
pub(super) struct ColorSvgRenders {
    pending: FxHashMap<RenderColorSvgParams, Task<()>>,
    failed: FxHashMap<RenderColorSvgParams, Instant>,
}

pub(super) fn rasterize_in_background(params: &RenderColorSvgParams) -> bool {
    i64::from(params.size.width.0) * i64::from(params.size.height.0) > MAX_SYNC_PIXELS
}

impl Window {
    pub(super) fn request_color_svg_raster(
        &mut self,
        params: RenderColorSvgParams,
        data: Option<&[u8]>,
        cx: &App,
    ) {
        let now = cx.background_executor().now();
        self.color_svg_renders
            .failed
            .retain(|_, failed_at| now.duration_since(*failed_at) < Duration::from_secs(1));
        if self.color_svg_renders.pending.contains_key(&params)
            || self.color_svg_renders.failed.contains_key(&params)
            || self.color_svg_renders.pending.len() >= MAX_CONCURRENT_RASTERS
        {
            return;
        }

        let renderer = cx.svg_renderer();
        let data = data.map(<[u8]>::to_vec);
        let render_params = params.clone();
        let raster = cx
            .background_executor()
            .spawn(async move { renderer.render_color_image(&render_params, data.as_deref()) });
        let key = params.clone();
        let task = self.spawn(cx, async move |cx| {
            let result = raster.await;
            let _ = cx.update(|window, cx| {
                window.color_svg_renders.pending.remove(&params);
                let uploaded = match result {
                    Ok(Some((size, bytes))) => {
                        let mut bytes = Some(bytes);
                        window
                            .sprite_atlas
                            .get_or_insert_with(&params.clone().into(), &mut || {
                                Ok(bytes.take().map(|bytes| (size, Cow::Owned(bytes))))
                            })
                    }
                    Ok(None) => Ok(None),
                    Err(error) => Err(error),
                };
                match uploaded {
                    Ok(Some(_)) => {}
                    result => {
                        if let Err(error) = result {
                            log::error!("Failed to render colored SVG {}: {error:#}", params.path);
                        }
                        // Back off missing or invalid sources without permanently caching errors.
                        window
                            .color_svg_renders
                            .failed
                            .insert(params, cx.background_executor().now());
                    }
                }
                // Refresh every consumer, including views that were painted while
                // the two worker slots were occupied. New paints request only their
                // current sizes; there is no queue of obsolete resize requests.
                window.refresh();
            });
        });
        self.color_svg_renders.pending.insert(key, task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppContext, AssetSource, AtlasTile, Context, Corners, DevicePixels, IntoElement, Render,
        Result, SMOOTH_SVG_SCALE_FACTOR, SharedString, Styled, SvgRenderer, TestAppContext,
        TransformationMatrix, canvas, div, point, px, rgb, size,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Assets(Arc<AtomicUsize>);
    impl AssetSource for Assets {
        fn load(&self, _: &str) -> Result<Option<Cow<'static, [u8]>>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Some(Cow::Borrowed(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="2" height="1" fill="currentColor"/></svg>"#)))
        }
        fn list(&self, _: &str) -> Result<Vec<SharedString>> {
            Ok(Vec::new())
        }
    }
    struct Empty;
    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    fn params(width: i32) -> RenderColorSvgParams {
        RenderColorSvgParams {
            path: "large.svg".into(),
            size: size(DevicePixels(width), DevicePixels(width / 2)),
            logical_size: size(px(width as f32), px(width as f32 / 2.)),
            object_fit: crate::ObjectFit::Contain,
            current_color: Some(rgb(0xff6633).into()),
            fill_color: None,
            text_color: None,
        }
    }
    fn cached(window: &Window, params: &RenderColorSvgParams) -> Option<AtlasTile> {
        window
            .sprite_atlas
            .get_or_insert_with(&params.clone().into(), &mut || Ok(None))
            .unwrap()
    }

    #[crate::test]
    fn small_svg_with_supplied_bytes_defers_first_paint_and_reuses_tile(cx: &mut TestAppContext) {
        let mut cx = cx.add_empty_window();
        let raster_width = cx.update(|window, _| {
            DevicePixels((32. * window.scale_factor() * SMOOTH_SVG_SCALE_FACTOR) as i32)
        });
        let p = RenderColorSvgParams {
            size: size(raster_width, raster_width),
            logical_size: size(px(32.), px(32.)),
            current_color: None,
            ..params(64)
        };
        for first_paint in [true, false] {
            let p = p.clone();
            cx.draw(point(px(0.), px(0.)), size(px(32.), px(32.)), |_, _| {
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.paint_color_svg(
                            bounds,
                            p.path.clone(),
                            Some(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>"#),
                            TransformationMatrix::default(),
                            Corners::default(),
                            None, None, None, cx,
                        ).unwrap();
                        if first_paint {
                            assert!(cached(window, &p).is_none(), "first paint must defer rasterization");
                            assert!(window.color_svg_renders.pending.contains_key(&p));
                        } else {
                            assert_eq!(cached(window, &p).unwrap().bounds.size, p.size);
                            assert!(window.color_svg_renders.pending.is_empty(), "cache hit must not restart work");
                        }
                    },
                ).size(px(32.))
            });
            cx.run_until_parked();
        }
    }

    #[crate::test]
    fn fitted_color_svg_async_cache_preserves_geometry_and_separates_modes(
        cx: &mut TestAppContext,
    ) {
        use crate::{ObjectFit, Transformation};
        let mut cx = cx.add_empty_window();
        let mut tiles = Vec::new();
        for fit in [
            ObjectFit::Fill,
            ObjectFit::Contain,
            ObjectFit::Cover,
            ObjectFit::ScaleDown,
            ObjectFit::None,
        ] {
            let p = cx.update(|window, _| RenderColorSvgParams {
                size: window
                    .snap_bounds(crate::Bounds::new(
                        point(px(12.), px(18.)),
                        size(px(90.), px(60.)),
                    ))
                    .size
                    .map(|p| DevicePixels((p.0 * SMOOTH_SVG_SCALE_FACTOR).ceil() as i32)),
                logical_size: size(px(90.), px(60.)),
                object_fit: fit,
                ..params(180)
            });
            for pass in 0..3 {
                let paint_params = p.clone();
                cx.draw(point(px(12.), px(18.)), size(px(90.), px(60.)), |_, _| {
                    canvas(|_, _, _| (), move |bounds, _, window, cx| {
                        let p = &paint_params;
                        window.next_frame.scene.clear();
                        let corners = Corners::all(px(8.));
                        let transform = Transformation::rotate(crate::radians(0.25))
                            .with_scaling(size(1.1, 0.9))
                            .into_matrix(bounds.center(), window.scale_factor());
                        window.paint_color_svg_with_fit(
                            bounds, p.path.clone(),
                            Some(br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="currentColor"/></svg>"#),
                            transform, corners, p.current_color, p.fill_color, p.text_color, fit, cx,
                        ).unwrap();
                        if pass == 0 {
                            assert!(cached(window, &p).is_none());
                            assert!(window.color_svg_renders.pending.contains_key(&p));
                            assert!(window.next_frame.scene.polychrome_sprites.is_empty());
                        } else {
                            assert!(window.color_svg_renders.pending.is_empty());
                            let sprite = &window.next_frame.scene.polychrome_sprites[0];
                            assert_eq!(sprite.bounds, window.snap_bounds(bounds));
                            assert_eq!(sprite.clip_bounds, sprite.bounds);
                            assert_eq!(sprite.corner_radii, corners.scale(window.scale_factor()));
                            assert_eq!(sprite.transformation, transform);
                            assert_eq!(sprite.tile.tile_id, cached(window, &p).unwrap().tile_id);
                        }
                    }).w(px(90.)).h(px(60.))
                });
                cx.run_until_parked();
                if pass > 0 {
                    let tile = cx.update(|window, _| cached(window, &p).unwrap());
                    if pass == 1 {
                        assert!(
                            !tiles.contains(&tile.tile_id),
                            "fit modes must not alias atlas tiles"
                        );
                        tiles.push(tile.tile_id);
                    } else {
                        assert_eq!(tiles.last(), Some(&tile.tile_id));
                    }
                }
            }
            cx.update(|window, _| {
                let mut other = p.clone();
                other.logical_size = other.logical_size / 2.;
                assert!(
                    cached(window, &other).is_none(),
                    "logical size is part of the raster key"
                );
            });
        }
    }

    #[crate::test]
    fn external_color_svg_default_contain_keeps_layout_through_loading(cx: &mut TestAppContext) {
        struct Source(std::path::PathBuf);
        impl Drop for Source {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let source = Source(std::env::temp_dir().join(format!(
                "gpui-color-svg-fit-{}-{}.svg",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        std::fs::write(&source.0, br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" fill="currentColor"/></svg>"#).unwrap();
        struct ExternalSvg(SharedString);
        impl Render for ExternalSvg {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                crate::color_svg()
                    .external_path(self.0.clone())
                    .current_color(rgb(0xff6633))
                    .size_full()
                    .rounded(px(12.))
            }
        }
        let window = cx.open_window(size(px(900.), px(600.)), |_, _| {
            ExternalSvg(source.0.to_string_lossy().into_owned().into())
        });
        let mut ready = Vec::new();
        for _ in 0..4 {
            cx.update_window(window.into(), |_, window, cx| {
                window.draw(cx).clear();
                if let Some(sprite) = window.rendered_frame.scene.polychrome_sprites.first() {
                    let bounds = window.snap_bounds(crate::Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(900.), px(600.)),
                    ));
                    assert_eq!(sprite.bounds, bounds);
                    let p = RenderColorSvgParams {
                        path: source.0.to_string_lossy().into_owned().into(),
                        size: bounds
                            .size
                            .map(|p| DevicePixels((p.0 * SMOOTH_SVG_SCALE_FACTOR).ceil() as i32)),
                        logical_size: size(px(900.), px(600.)),
                        ..params(1800)
                    };
                    assert_eq!(cached(window, &p).unwrap().tile_id, sprite.tile.tile_id);
                    ready.push(*sprite);
                }
            })
            .unwrap();
            cx.run_until_parked();
        }
        assert_eq!(
            ready.len(),
            2,
            "file loading and rasterization each defer one paint"
        );
        assert_eq!(
            ready[0], ready[1],
            "first ready paint and cache hit must agree"
        );
    }

    #[crate::test]
    fn large_color_svg_raster_is_deferred_deduplicated_and_uploaded(cx: &mut TestAppContext) {
        let loads = Arc::new(AtomicUsize::new(0));
        cx.update(|cx| cx.svg_renderer = SvgRenderer::new(Arc::new(Assets(loads.clone()))));
        let window = cx.open_window(size(px(800.), px(600.)), |_, _| Empty);
        let p = params(900);
        window
            .update(cx, |_, window, cx| {
                assert!(rasterize_in_background(&p));
                window.request_color_svg_raster(p.clone(), None, cx);
                window.request_color_svg_raster(p.clone(), None, cx);
                assert_eq!(
                    loads.load(Ordering::SeqCst),
                    0,
                    "paint must not load or rasterize synchronously"
                );
                assert!(cached(window, &p).is_none());
                assert_eq!(window.color_svg_renders.pending.len(), 1);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |_, window, _| {
                let tile = cached(window, &p).unwrap();
                assert_eq!(tile.bounds.size, p.size);
                assert!(window.color_svg_renders.pending.is_empty());
                assert_eq!(loads.load(Ordering::SeqCst), 1);
                assert_eq!(cached(window, &p).unwrap().tile_id, tile.tile_id);
            })
            .unwrap();
    }

    #[crate::test]
    fn resizing_bounds_worker_count_and_does_not_mix_color_variants(cx: &mut TestAppContext) {
        let loads = Arc::new(AtomicUsize::new(0));
        cx.update(|cx| cx.svg_renderer = SvgRenderer::new(Arc::new(Assets(loads.clone()))));
        let window = cx.open_window(size(px(800.), px(600.)), |_, _| Empty);
        let a = params(600);
        let b = params(800);
        let mut latest = params(1000);
        latest.current_color = Some(rgb(0x3366ff).into());
        window
            .update(cx, |_, window, cx| {
                for p in [&a, &b, &latest] {
                    window.request_color_svg_raster(p.clone(), None, cx);
                }
                assert_eq!(window.color_svg_renders.pending.len(), 2);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |_, window, cx| {
                assert!(cached(window, &latest).is_none());
                assert_eq!(cached(window, &a).unwrap().bounds.size, a.size);
                assert_eq!(cached(window, &b).unwrap().bounds.size, b.size);
                window.request_color_svg_raster(latest.clone(), None, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |_, window, _| {
                assert_eq!(cached(window, &latest).unwrap().bounds.size, latest.size);
                let mut other = latest.clone();
                other.current_color = a.current_color;
                assert!(cached(window, &other).is_none());
                assert_eq!(loads.load(Ordering::SeqCst), 3);
            })
            .unwrap();
    }

    #[crate::test]
    fn invalid_large_svg_does_not_restart_on_every_redraw(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, _| Empty);
        let p = params(900);
        window
            .update(cx, |_, window, cx| {
                window.request_color_svg_raster(p.clone(), Some(b"invalid"), cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(cx, |_, window, cx| {
                window.request_color_svg_raster(p.clone(), Some(b"invalid"), cx);
                assert!(window.color_svg_renders.pending.is_empty());
                assert!(window.color_svg_renders.failed.contains_key(&p));
            })
            .unwrap();
        cx.executor().advance_clock(Duration::from_secs(2));
        window.update(cx, |_, window, cx| {
            window.request_color_svg_raster(p.clone(), Some(br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="2" height="1"/></svg>"#), cx);
        }).unwrap();
        cx.run_until_parked();
        window
            .update(cx, |_, window, _| assert!(cached(window, &p).is_some()))
            .unwrap();
    }
}
