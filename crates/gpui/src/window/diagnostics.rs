use super::Window;
use std::time::Duration;

/// Cache decisions actually visited during a frame. A reused parent skips its descendants.
#[derive(Clone, Copy, Debug, Default)]
pub struct CacheDiagnostics {
    /// Visited entries whose existing content was reused.
    pub hits: u64,
    /// Visited entries whose content had to be generated.
    pub misses: u64,
}

#[cfg(test)]
mod tests {
    use crate::{AppContext, Context, IntoElement, Render, TestAppContext, Window, div};

    struct Content;

    impl Render for Content {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[crate::test]
    fn submitted_diagnostics_survive_builds_until_platform_draw(cx: &mut TestAppContext) {
        let handle = cx.add_window(|_, _| Content);
        cx.update_window(handle.into(), |_, window, cx| {
            window.set_frame_diagnostics_enabled(true);
            window.draw(cx).clear();
            assert!(window.submitted_frame_diagnostics().is_none());
            window.present();
            let first = window.submitted_frame_diagnostics().unwrap().sequence;
            assert!(
                window
                    .submitted_frame_diagnostics()
                    .unwrap()
                    .platform_draw_time
                    .is_some()
            );

            for _ in 0..2 {
                window.draw(cx).clear();
                assert!(window.frame_diagnostics().unwrap().sequence > first);
                assert!(
                    window
                        .frame_diagnostics()
                        .unwrap()
                        .platform_draw_time
                        .is_none()
                );
                assert_eq!(
                    window.submitted_frame_diagnostics().unwrap().sequence,
                    first
                );
            }

            window.present();
            let submitted = window.submitted_frame_diagnostics().unwrap();
            assert_eq!(
                submitted.sequence,
                window.frame_diagnostics().unwrap().sequence
            );
            assert!(submitted.sequence > first);
            assert!(submitted.platform_draw_time.is_some());
            // The test platform has no renderer diagnostics; absence must remain visible.
            assert!(submitted.renderer.is_none());

            window.set_frame_diagnostics_enabled(false);
            assert!(window.frame_diagnostics().is_none());
            assert!(window.submitted_frame_diagnostics().is_none());
        })
        .unwrap();
    }
}

/// Reasons a cached view was rebuilt. Categories are exclusive, in field order.
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewCacheMisses {
    /// No previous cache state exists.
    pub cold: u64,
    /// Accessibility requires a fresh tree.
    pub accessibility: u64,
    /// A full-window or ancestor refresh requires repainting.
    pub refresh: u64,
    /// The view or a dependency was invalidated.
    pub dirty: u64,
    /// Bounds, style, density, clipping or coordinate scope changed.
    pub context: u64,
}

/// One retained UI capture or subtree scratch texture, excluding driver overhead.
#[derive(Clone, Debug)]
pub struct CaptureTextureDiagnostics {
    /// Physical texture width.
    pub width: u32,
    /// Physical texture height.
    pub height: u32,
    /// Uncompressed texel storage estimate; not measured GPU memory usage.
    pub estimated_bytes: u64,
    /// Local raster multiplier for an isolated UI capture; `None` for scratch targets.
    /// Nested multipliers compose, and window device density is additional.
    pub raster_scale: Option<f32>,
}

/// WGPU's latest scene encoding attempt and currently retained capture targets.
#[derive(Clone, Debug, Default)]
pub struct RendererDiagnostics {
    /// Source capture reuse decisions, including nested captures actually encoded.
    pub capture_cache: CacheDiagnostics,
    /// Retained capture textures across nested renderers. Excludes atlas, path/MSAA,
    /// backdrop, media, 3D, simulation textures and presentation buffers.
    pub capture_textures: Vec<CaptureTextureDiagnostics>,
}

/// Last completed window scene build. Timings are CPU wall time, not GPU execution time.
#[derive(Clone, Debug, Default)]
pub struct FrameDiagnostics {
    /// Monotonically increasing scene-build number while tracking is enabled.
    pub sequence: u64,
    /// Logical viewport at scene-build completion.
    pub viewport_size: crate::Size<crate::Pixels>,
    /// Window device-pixel scale at scene-build completion.
    pub scale_factor: f32,
    /// Scene construction, including any raster-budget retry.
    pub build_time: Duration,
    /// Time spent in the platform draw call, including encoding/submission or surface waits.
    /// `None` until the scene has been passed to the platform; does not prove presentation.
    pub platform_draw_time: Option<Duration>,
    /// Number of scene builds, including raster-budget retries, for this frame.
    pub build_attempts: u32,
    /// Decisions for explicitly cached views. Uncached and Inspector-bypassed views are excluded.
    pub view_cache: CacheDiagnostics,
    /// Exclusive rebuild reasons, summing to `view_cache.misses`.
    pub view_cache_misses: ViewCacheMisses,
    /// Available after platform draw on instrumented renderers (currently Linux/Web WGPU).
    pub renderer: Option<RendererDiagnostics>,
}

#[derive(Default)]
pub(crate) struct FrameDiagnosticsTracker {
    pub current: FrameDiagnostics,
    pub completed: Option<FrameDiagnostics>,
    pub submitted: Option<FrameDiagnostics>,
}

impl Window {
    pub(super) fn finish_frame_diagnostics(&mut self, elapsed: Duration) {
        if let Some(tracker) = &mut self.frame_diagnostics {
            tracker.current.build_time = elapsed;
            tracker.current.viewport_size = self.viewport_size;
            tracker.current.scale_factor = self.scale_factor;
            tracker.completed = Some(tracker.current.clone());
        }
    }

    /// Enables per-window diagnostics. Disabled by default; disabling releases the snapshots.
    pub fn set_frame_diagnostics_enabled(&mut self, enabled: bool) {
        if enabled {
            self.frame_diagnostics.get_or_insert_with(Default::default);
        } else {
            self.frame_diagnostics = None;
        }
    }

    /// Returns the last completed build without requesting a redraw or waiting for the GPU.
    pub fn frame_diagnostics(&self) -> Option<&FrameDiagnostics> {
        self.frame_diagnostics.as_ref()?.completed.as_ref()
    }

    /// Returns the last scene passed to platform draw, preserving it across pending builds.
    /// CPU and renderer statistics refer to the same scene. Renderer details may still be
    /// unavailable if platform drawing failed or was skipped; this does not prove presentation.
    pub fn submitted_frame_diagnostics(&self) -> Option<&FrameDiagnostics> {
        self.frame_diagnostics.as_ref()?.submitted.as_ref()
    }

    pub(crate) fn record_view_cache_hit(&mut self) {
        if let Some(tracker) = &mut self.frame_diagnostics {
            tracker.current.view_cache.hits += 1;
        }
    }

    pub(crate) fn record_view_cache_miss(&mut self, cold: bool, entity: crate::EntityId) {
        if let Some(tracker) = &mut self.frame_diagnostics {
            tracker.current.view_cache.misses += 1;
            let reasons = &mut tracker.current.view_cache_misses;
            if cold {
                reasons.cold += 1;
            } else if self.a11y.is_active() {
                reasons.accessibility += 1;
            } else if self.refreshing {
                reasons.refresh += 1;
            } else if self.dirty_views.contains(&entity) {
                reasons.dirty += 1;
            } else {
                reasons.context += 1;
            }
        }
    }
}
