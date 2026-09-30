use super::{ContentMask, MaskedPaint, Window};
use crate::{
    AtlasTile, BackdropBlur, Background, Bounds, Corners, EffectQuad, EffectShader, EffectUniforms,
    PaintBackdropBlur, PaintBackdropEffect, PaintEffect, Pixels, Point, RenderImage,
    RenderImageParams, ScaledPixels, TransformationMatrix, px,
};
use anyhow::{Result, anyhow};
use std::{borrow::Cow, sync::Arc};

impl Window {
    /// Overrides the fill used by monochrome content painted by `f`.
    ///
    /// Text glyphs and monochrome SVGs use their alpha atlas as a mask for the
    /// supplied background. The background is sampled relative to `bounds`, so
    /// separate glyph sprites share one continuous gradient instead of
    /// restarting it for every glyph.
    ///
    /// Polychrome content such as emoji, images, and [`ColorSvg`](crate::ColorSvg)
    /// is intentionally unaffected.
    pub fn with_masked_fill<F, R>(
        &mut self,
        bounds: Bounds<Pixels>,
        background: impl Into<Background>,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.invalidator.debug_assert_paint();
        self.masked_paint_stack.push(MaskedPaint::Fill {
            background: background.into(),
            bounds,
        });
        let result = f(self);
        self.masked_paint_stack.pop();
        result
    }

    /// Evaluates a custom fragment effect through monochrome content painted by `f`.
    ///
    /// `bounds` defines one shared coordinate system for every glyph or SVG
    /// mask produced by the closure. The shader must have been created with
    /// [`EffectShader::wgsl_mask`].
    #[allow(clippy::too_many_arguments)]
    pub fn with_masked_effect<F, R>(
        &mut self,
        bounds: Bounds<Pixels>,
        shader: EffectShader,
        uniforms: EffectUniforms,
        time: f32,
        opacity: f32,
        f: F,
    ) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        self.invalidator.debug_assert_paint();
        assert!(
            shader.is_mask(),
            "masked effects require EffectShader::wgsl_mask"
        );
        self.masked_paint_stack.push(MaskedPaint::Effect {
            bounds,
            shader,
            uniforms,
            time,
            opacity: opacity.clamp(0.0, 1.0),
        });
        let result = f(self);
        self.masked_paint_stack.pop();
        result
    }

    /// Returns whether isolated subtree effects are available on this window.
    pub fn supports_subtree_effects(&self) -> bool {
        self.platform_window.supports_subtree_effects()
    }

    /// Whether this window supports GPU particle simulation and drawing.
    pub fn supports_gpu_particles(&self) -> bool {
        self.platform_window.supports_gpu_particles()
    }

    /// Whether this window supports GPU fluid simulation and drawing.
    pub fn supports_gpu_fluid(&self) -> bool {
        self.platform_window.supports_gpu_fluid()
    }

    /// Whether depth-tested mesh viewports are available on this window.
    pub fn supports_scene3d(&self) -> bool {
        self.platform_window.supports_scene3d()
    }

    /// Current renderer capabilities or the reason mesh viewports are unavailable.
    /// Query again after renderer/device replacement; this does not schedule a frame.
    pub fn scene3d_support(&self) -> crate::Scene3dSupport {
        self.platform_window.scene3d_support()
    }

    /// Returns the current backend context for device-local resource extensions.
    /// The concrete type is backend-specific and may change after device recovery.
    pub fn renderer_context(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        self.sprite_atlas.renderer_context()
    }

    /// Releases mesh-rendering caches for all 3D viewports in this window.
    /// Shared 2D atlas and UI capture resources remain valid. The next mesh draw
    /// rebuilds caches lazily; this does not schedule a frame or wait for the GPU.
    /// Unsupported backends do nothing.
    pub fn clear_scene3d_caches(&mut self) {
        self.platform_window.clear_scene3d_caches();
    }

    /// Mesh output-cache allocations across this window and its UI captures.
    /// Unsupported backends return `None`. This is not a total GPU-memory report.
    pub fn scene3d_output_cache_stats(&self) -> Option<crate::Scene3dOutputCacheStats> {
        self.platform_window.scene3d_output_cache_stats()
    }

    /// Sets the shared mesh output-cache budget in bytes; zero disables mesh pixel reuse.
    /// A changed budget releases existing entries. Shared atlas and UI textures remain
    /// valid. Does not request a frame or wait for the GPU; unsupported backends do nothing.
    pub fn set_scene3d_output_cache_budget(&mut self, bytes: u64) {
        self.platform_window.set_scene3d_output_cache_budget(bytes);
    }

    /// Draws UI in texture-local coordinates at its own raster density.
    /// Use the same configuration during prepaint and paint inside a 3D capture.
    /// Ancestor masks apply to the final viewport, not to the source texture.
    pub fn with_scene3d_texture<R>(
        &mut self,
        texture: crate::UiTexture3d,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint_or_prepaint();
        let scale = std::mem::replace(&mut self.scale_factor, texture.scale_factor());
        let raster = std::mem::replace(&mut self.subtree_raster_scale, 1.);
        let masks = std::mem::replace(
            &mut self.content_mask_stack,
            vec![ContentMask {
                bounds: Bounds::new(Point::default(), texture.logical_size()),
            }],
        );
        let result = f(self);
        self.content_mask_stack = masks;
        self.scale_factor = scale;
        self.subtree_raster_scale = raster;
        result
    }

    /// Captures a decorative UI texture and renders a depth-tested mesh scene.
    /// Prepaint the texture with `prepaint_subtree_effect`. Unsupported platforms
    /// draw nothing. The caller owns input routing and animation scheduling.
    pub fn with_scene3d(
        &mut self,
        bounds: Bounds<Pixels>,
        frame: Arc<crate::Scene3dFrame>,
        paint_texture: impl FnOnce(&mut Self),
    ) {
        self.invalidator.debug_assert_paint();
        if !self.supports_scene3d() || bounds.is_empty() || self.element_opacity <= 0. {
            return;
        }
        self.with_subtree_effect(
            bounds,
            EffectShader::wgsl_image("fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }"),
            EffectUniforms::default(), 0., 1.,
            |window| {
                window.next_frame.scene.set_subtree_scene3d(frame);
                paint_texture(window);
            },
        );
    }

    /// Paints a fluid surface. Unsupported renderers do not draw the surface.
    pub fn paint_fluid(&mut self, bounds: Bounds<Pixels>, frame: Arc<crate::FluidFrame>) {
        self.invalidator.debug_assert_paint();
        if !self.supports_gpu_fluid() {
            return;
        }
        let bounds = self.snap_bounds(bounds);
        let content_mask = self.snapped_content_mask();
        if self.element_opacity <= 0. || bounds.intersect(&content_mask.bounds).is_empty() {
            return;
        }
        if frame.needs_animation {
            self.request_animation_frame();
        }
        self.next_frame.scene.insert_primitive(crate::FluidDraw {
            order: 0,
            bounds,
            content_mask,
            scale_factor: self.raster_scale_factor(),
            opacity: self.element_opacity,
            frame,
        });
    }

    /// Paints a particle surface. Unsupported renderers do not draw particles.
    pub fn paint_particles(&mut self, bounds: Bounds<Pixels>, frame: Arc<crate::ParticleFrame>) {
        self.invalidator.debug_assert_paint();
        if !self.supports_gpu_particles() {
            return;
        }
        let bounds = self.snap_bounds(bounds);
        let content_mask = self.snapped_content_mask();
        if self.element_opacity <= 0. || bounds.intersect(&content_mask.bounds).is_empty() {
            return;
        }
        if frame.needs_animation {
            self.request_animation_frame();
        }
        self.next_frame.scene.insert_primitive(crate::ParticleDraw {
            order: 0,
            bounds,
            content_mask,
            scale_factor: self.raster_scale_factor(),
            opacity: self.element_opacity,
            frame,
        });
    }

    /// Prepaints content that will be painted with [`Self::with_subtree_effect`].
    ///
    /// Cached views distinguish transparent capture from normal window rendering.
    pub fn prepaint_subtree_effect<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.invalidator.debug_assert_prepaint();
        let previous = self.prepainting_subtree_effect;
        self.prepainting_subtree_effect |= self.supports_subtree_effects();
        let result = f(self);
        self.prepainting_subtree_effect = previous;
        result
    }

    /// Captures the primitives painted by `f` and composites them through an image shader.
    ///
    /// The shader samples straight-alpha colors with `sample_effect_image`.
    /// Samples outside `bounds` are transparent. Layout and hit testing remain
    /// in their original coordinates; shader displacement is visual only.
    /// Platforms without subtree support paint the closure normally.
    /// Prepaint cached content inside [`Self::prepaint_subtree_effect`].
    #[allow(clippy::too_many_arguments)]
    pub fn with_subtree_effect<R>(
        &mut self,
        bounds: Bounds<Pixels>,
        shader: EffectShader,
        uniforms: EffectUniforms,
        time: f32,
        opacity: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.with_subtree_effect_chain(
            bounds,
            &[crate::SubtreeEffectPass {
                shader,
                uniforms,
                time,
                images: Default::default(),
                bloom: None,
                feedback: None,
                distance_field: None,
                particles: None,
                particle_transition: None,
            }],
            opacity,
            f,
        )
    }

    /// Captures content once and processes it through ordered image passes.
    ///
    /// Opacity applies only to the final composite. An empty chain paints directly.
    /// Prepaint cached content inside [`Self::prepaint_subtree_effect`].
    pub fn with_subtree_effect_chain<R>(
        &mut self,
        bounds: Bounds<Pixels>,
        passes: &[crate::SubtreeEffectPass],
        opacity: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.invalidator.debug_assert_paint();
        assert!(
            passes
                .iter()
                .all(
                    |pass| usize::from(pass.shader.image_count()) == 1 + pass.images.len()
                        && !pass.shader.is_mask()
                        && (pass.images.is_empty()
                            || (pass.bloom.is_none()
                                && pass.feedback.is_none()
                                && pass.distance_field.is_none()
                                && pass.particles.is_none()
                                && pass.particle_transition.is_none()))
                        && pass.bloom.as_ref().is_none_or(|bloom| {
                            bloom.extract.image_count() == 1
                                && !bloom.extract.is_mask()
                                && bloom.blur.image_count() == 1
                                && !bloom.blur.is_mask()
                                && bloom.composite.image_count() == 2
                                && !bloom.composite.is_mask()
                        })
                        && pass.feedback.as_ref().is_none_or(|feedback| {
                            pass.bloom.is_none()
                                && feedback.shader.image_count() == 2
                                && !feedback.shader.is_mask()
                        })
                        && pass.distance_field.as_ref().is_none_or(|field| {
                            pass.bloom.is_none()
                                && pass.feedback.is_none()
                                && field.composite.image_count() == 2
                                && !field.composite.is_mask()
                        })
                        && pass.particles.as_ref().is_none_or(|_| {
                            pass.bloom.is_none()
                                && pass.feedback.is_none()
                                && pass.distance_field.is_none()
                        })
                        && pass.particle_transition.as_ref().is_none_or(|_| {
                            pass.bloom.is_none()
                                && pass.feedback.is_none()
                                && pass.distance_field.is_none()
                                && pass.particles.is_none()
                        })
                ),
            "invalid shader inputs for subtree effect passes"
        );
        if passes.is_empty() || !self.supports_subtree_effects() {
            return self.with_element_opacity(Some(opacity.clamp(0.0, 1.0)), f);
        }
        let (last, intermediate) = passes.split_last().unwrap();
        let intermediate = if last.bloom.is_some()
            || last.feedback.is_some()
            || last.distance_field.is_some()
            || last.particles.is_some()
            || last.particle_transition.is_some()
            || !last.images.is_empty()
        {
            passes
        } else {
            intermediate
        };
        let bounds = self.snap_bounds(bounds);
        let previous_opacity = self.element_opacity;
        self.element_opacity = 1.0;
        let composite = EffectQuad {
            order: 0,
            bounds,
            effect_bounds: bounds,
            transformation: TransformationMatrix::default(),
            content_mask: self.snapped_content_mask(),
            shader: if last.images.is_empty() {
                last.shader.clone()
            } else {
                EffectShader::wgsl_image(
                    "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
                )
            },
            uniforms: last.uniforms,
            time: last.time,
            corner_radii: Corners::default(),
            opacity: previous_opacity * opacity.clamp(0.0, 1.0),
            image_tile: None,
            second_image_tile: None,
            third_image_tile: None,
            fourth_image_tile: None,
        };
        if intermediate.is_empty() {
            self.next_frame.scene.start_subtree(composite);
        } else {
            self.next_frame
                .scene
                .start_subtree_chain(composite, intermediate.into());
        }
        let result = f(self);
        self.next_frame.scene.end_subtree();
        self.element_opacity = previous_opacity;
        result
    }

    /// Captures two subtrees independently and composites them with a two-image shader.
    /// Both inputs share snapped capture bounds. Opacity applies once to the final output.
    /// Prepaint each input inside [`Self::prepaint_subtree_effect`]. Backends without
    /// subtree support paint only the second input. Input routing is controlled by the caller.
    #[allow(clippy::too_many_arguments)]
    pub fn with_subtree_pair(
        &mut self,
        bounds: Bounds<Pixels>,
        shader: EffectShader,
        uniforms: EffectUniforms,
        time: f32,
        opacity: f32,
        mut paint: impl FnMut(crate::SubtreeInput, &mut Self),
    ) {
        self.invalidator.debug_assert_paint();
        assert!(
            shader.image_count() == 2 && !shader.is_mask(),
            "subtree pairs require a two-image shader"
        );
        if !self.supports_subtree_effects() {
            self.with_element_opacity(Some(opacity.clamp(0., 1.)), |window| {
                paint(crate::SubtreeInput::Second, window)
            });
            return;
        }
        let bounds = self.snap_bounds(bounds);
        let previous_opacity = self.element_opacity;
        self.element_opacity = 1.;
        self.next_frame.scene.start_subtree(EffectQuad {
            order: 0,
            bounds,
            effect_bounds: bounds,
            transformation: TransformationMatrix::default(),
            content_mask: self.snapped_content_mask(),
            shader,
            uniforms,
            time,
            corner_radii: Corners::default(),
            opacity: previous_opacity * opacity.clamp(0., 1.),
            image_tile: None,
            second_image_tile: None,
            third_image_tile: None,
            fourth_image_tile: None,
        });
        paint(crate::SubtreeInput::First, self);
        self.next_frame.scene.next_subtree_input();
        paint(crate::SubtreeInput::Second, self);
        self.next_frame.scene.end_subtree();
        self.element_opacity = previous_opacity;
    }

    /// Paints a blur of all scene primitives that precede this call in draw order.
    ///
    /// The blur is clipped to `backdrop.bounds`, the current content mask, and the
    /// supplied corner radii. Renderers that do not support backdrop capture treat
    /// this operation as a no-op, so applications should paint their translucent
    /// fallback material separately.
    pub fn paint_backdrop_blur(&mut self, backdrop: PaintBackdropBlur) {
        self.invalidator.debug_assert_paint();

        let opacity = self.element_opacity() * backdrop.opacity.clamp(0.0, 1.0);
        let blur_radius = backdrop.blur_radius.max(px(0.));
        if opacity == 0.0 || blur_radius == px(0.) {
            return;
        }

        self.next_frame.scene.insert_primitive(BackdropBlur {
            order: 0,
            bounds: self.snap_bounds(backdrop.bounds),
            content_mask: self.snapped_content_mask(),
            corner_radii: backdrop.corner_radii.scale(self.raster_scale_factor()),
            blur_radius: ScaledPixels(blur_radius.0 * self.raster_scale_factor()),
            opacity,
            shader: None,
            uniforms: EffectUniforms::default(),
            time: 0.0,
            pointer: Point { x: 0.5, y: 0.5 },
            pointer_active: false,
        });
    }

    /// Paints a custom shader that samples raw and blurred scene content behind it.
    pub fn paint_backdrop_effect(&mut self, effect: PaintBackdropEffect) {
        self.invalidator.debug_assert_paint();

        let opacity = self.element_opacity() * effect.opacity.clamp(0.0, 1.0);
        if opacity == 0.0 {
            return;
        }
        let blur_radius = effect.blur_radius.max(px(0.));
        self.next_frame.scene.insert_primitive(BackdropBlur {
            order: 0,
            bounds: self.snap_bounds(effect.bounds),
            content_mask: self.snapped_content_mask(),
            corner_radii: effect.corner_radii.scale(self.raster_scale_factor()),
            blur_radius: ScaledPixels(blur_radius.0 * self.raster_scale_factor()),
            opacity,
            shader: Some(effect.shader),
            uniforms: effect.uniforms,
            time: effect.time,
            pointer: effect.pointer,
            pointer_active: effect.pointer_active,
        });
    }

    /// Resolves a decoded image frame to an atlas tile for an effect input.
    /// Call during paint. Images retain their straight-alpha BGRA data.
    pub fn prepare_effect_image(
        &mut self,
        image: &RenderImage,
        frame_index: usize,
    ) -> Result<AtlasTile> {
        self.invalidator.debug_assert_paint();
        let bytes = image
            .as_bytes(frame_index)
            .ok_or_else(|| anyhow!("image frame index is out of bounds"))?;
        let size = image.size(frame_index);
        if size.width.0 <= 0 || size.height.0 <= 0 {
            return Err(anyhow!("effect image is empty"));
        }
        let params = RenderImageParams {
            image_id: image.id,
            frame_index,
        };
        self.sprite_atlas
            .get_or_insert_with(&params.into(), &mut || {
                Ok(Some((size, Cow::Borrowed(bytes))))
            })?
            .ok_or_else(|| anyhow!("effect image has no atlas tile"))
    }

    /// Paints a custom fragment effect into the scene for the next frame.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_effect(&mut self, effect: PaintEffect) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let mut insert_effect_image =
            |source: &Option<(Arc<RenderImage>, usize)>, label: &str| -> Result<AtlasTile> {
                let (image, frame_index) = source.as_ref().ok_or_else(|| {
                    anyhow!("{label} image is required before this effect can be painted")
                })?;
                self.prepare_effect_image(image, *frame_index)
            };
        let image_tile = if effect.shader.image_count() >= 1 {
            Some(insert_effect_image(&effect.image, "primary")?)
        } else {
            None
        };
        let second_image_tile = if effect.shader.image_count() >= 2 {
            Some(insert_effect_image(&effect.second_image, "second")?)
        } else {
            None
        };
        let third_image_tile = if effect.shader.image_count() >= 4 {
            Some(insert_effect_image(&effect.third_image, "third")?)
        } else {
            None
        };
        let fourth_image_tile = if effect.shader.image_count() >= 4 {
            Some(insert_effect_image(&effect.fourth_image, "fourth")?)
        } else {
            None
        };
        let opacity = self.element_opacity() * effect.opacity;
        let snapped_bounds = self.snap_bounds(effect.bounds);
        self.next_frame.scene.insert_primitive(EffectQuad {
            order: 0,
            bounds: snapped_bounds,
            effect_bounds: snapped_bounds,
            transformation: TransformationMatrix::unit(),
            content_mask: self.snapped_content_mask(),
            shader: effect.shader,
            uniforms: effect.uniforms,
            time: effect.time,
            corner_radii: effect.corner_radii.scale(self.raster_scale_factor()),
            opacity,
            image_tile,
            second_image_tile,
            third_image_tile,
            fourth_image_tile,
        });
        Ok(())
    }
}
