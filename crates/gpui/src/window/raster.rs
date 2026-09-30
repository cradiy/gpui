use super::{DrawPhase, Window};
use crate::{Bounds, Pixels, ScaledPixels, Size, scene::RasterCaptureRegion};
use collections::FxHashSet;

fn region_key(bounds: Bounds<Pixels>) -> [u32; 4] {
    [
        bounds.origin.x.0,
        bounds.origin.y.0,
        bounds.size.width.0,
        bounds.size.height.0,
    ]
    .map(f32::to_bits)
}

fn limited_scale(multiplier: f32, size: Size<ScaledPixels>, padding: f32) -> f32 {
    let width = size.width.0.ceil().max(1.);
    let height = size.height.0.ceil().max(1.);
    let mut scale = multiplier
        .min(4.)
        .min(8192. / width.max(height))
        .min((16_777_216. / (width * height)).sqrt())
        .max(1.);
    let fits = |scale: f32| {
        let width = f64::from((width * scale).ceil() + padding);
        let height = f64::from((height * scale).ceil() + padding);
        width <= 8192. && height <= 8192. && width * height <= 16_777_216.
    };
    if scale > 1. && !fits(scale) {
        // Positive f32 bits are ordered. Integer bisection cannot stall when a
        // rounded pixel decrement maps back to the same floating-point value.
        let mut lower = 1_f32.to_bits();
        let mut upper = scale.to_bits();
        while upper - lower > 1 {
            let middle = lower + (upper - lower) / 2;
            if fits(f32::from_bits(middle)) {
                lower = middle;
            } else {
                upper = middle;
            }
        }
        scale = f32::from_bits(lower);
    }
    scale
}

impl Window {
    /// Paints an isolated subtree at a higher raster density without changing layout or input.
    /// Use inside both `prepaint_subtree_effect` and a single-pass `with_subtree_effect`.
    /// The multiplier must be finite and at least one. It is capped at four and reduced
    /// using the full viewport to fit 8192 pixels per axis and 16,777,216 pixels.
    /// Native density is never reduced. Unsupported backends keep normal density.
    pub fn with_subtree_raster_scale<R>(
        &mut self,
        multiplier: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.with_raster_capture(None, multiplier, f)
    }

    /// Applies capture density with a budget based on the subtree's visible source bounds.
    /// Pass the same bounds and multiplier during prepaint and paint, inside a single-pass
    /// subtree effect. Compatible WGPU captures use region limits; viewport-sized effects
    /// use full-window limits. A change requiring full-window capture may redraw the frame
    /// at safe density before presentation. Native density is never reduced.
    pub fn with_subtree_raster_scale_in<R>(
        &mut self,
        bounds: Bounds<Pixels>,
        multiplier: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.with_raster_capture(Some(bounds), multiplier, f)
    }

    fn with_raster_capture<R>(
        &mut self,
        bounds: Option<Bounds<Pixels>>,
        multiplier: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint_or_prepaint();
        assert!(multiplier.is_finite() && multiplier >= 1.);
        if !self.supports_subtree_effects() {
            return f(self);
        }
        let viewport = self.viewport_size().scale(self.raster_scale_factor());
        let full_viewport_scale = limited_scale(multiplier, viewport, 0.);
        let region = bounds.map(|bounds| {
            let visible = self
                .snap_bounds(bounds)
                .intersect(&Bounds::new(Default::default(), viewport));
            RasterCaptureRegion {
                bounds,
                full_viewport_scale,
                // Floor/ceil of a fractional origin can add one pixel per axis.
                region_scale: limited_scale(multiplier, visible.size, 1.),
            }
        });
        let scale = region
            .as_ref()
            .filter(|region| {
                !self.raster_budget_retrying
                    && !self
                        .raster_full_viewport_regions
                        .contains(&region_key(region.bounds))
            })
            .map_or(full_viewport_scale, |region| region.region_scale);
        if self.invalidator.inner.borrow().draw_phase == DrawPhase::Paint {
            self.next_frame
                .scene
                .set_subtree_raster_capture(scale, region);
        }
        let previous = self.subtree_raster_scale;
        self.subtree_raster_scale *= scale;
        let result = f(self);
        self.subtree_raster_scale = previous;
        result
    }

    pub(super) fn update_raster_capture_budgets(&mut self) -> (bool, bool) {
        let mut full_viewport = FxHashSet::default();
        let mut retry = false;
        self.rendered_frame.scene.visit(&mut |scene| {
            if let Some(region) = scene.raster_region
                && !scene.supports_region_capture()
            {
                full_viewport.insert(region_key(region.bounds));
                retry |= scene.raster_scale.unwrap_or(1.) > region.full_viewport_scale;
            }
        });
        let mut upgrade = false;
        self.rendered_frame.scene.visit(&mut |scene| {
            if let Some(region) = scene.raster_region {
                upgrade |= !full_viewport.contains(&region_key(region.bounds))
                    && scene.raster_scale.unwrap_or(1.) < region.region_scale;
            }
        });
        self.raster_full_viewport_regions = full_viewport;
        (retry, upgrade)
    }
}
