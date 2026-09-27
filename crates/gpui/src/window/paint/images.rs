use super::super::{MaskedPaint, Window, color_svg};
use crate::util::round_half_toward_zero;
use crate::{
    App, Background, Bounds, Corners, DevicePixels, EffectQuad, Hsla, MonochromeSprite, ObjectFit,
    Pixels, Point, PolychromeSprite, RenderColorSvgParams, RenderImage, RenderImageParams,
    RenderSvgParams, SMOOTH_SVG_SCALE_FACTOR, ScaledPixels, SharedString, TransformationMatrix,
};
use anyhow::Result;
use std::{borrow::Cow, sync::Arc};

impl Window {
    /// Paint a monochrome SVG into the scene for the next frame at the current stacking context.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_svg(
        &mut self,
        bounds: Bounds<Pixels>,
        path: SharedString,
        mut data: Option<&[u8]>,
        transformation: TransformationMatrix,
        color: Hsla,
        cx: &App,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let element_opacity = self.element_opacity();
        let bounds = self.snap_bounds(bounds);

        let params = RenderSvgParams {
            path,
            size: bounds.size.map(|pixels| {
                DevicePixels::from((pixels.0 * SMOOTH_SVG_SCALE_FACTOR).ceil() as i32)
            }),
        };

        let Some(tile) =
            self.sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    let Some((size, bytes)) = cx.svg_renderer.render_alpha_mask(&params, data)?
                    else {
                        return Ok(None);
                    };
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
        else {
            return Ok(());
        };
        let content_mask = self.snapped_content_mask();
        let svg_bounds = Bounds {
            origin: bounds.center()
                - Point::new(
                    ScaledPixels(tile.bounds.size.width.0 as f32 / SMOOTH_SVG_SCALE_FACTOR / 2.),
                    ScaledPixels(tile.bounds.size.height.0 as f32 / SMOOTH_SVG_SCALE_FACTOR / 2.),
                ),
            size: tile
                .bounds
                .size
                .map(|value| ScaledPixels(value.0 as f32 / SMOOTH_SVG_SCALE_FACTOR)),
        };
        let final_bounds = svg_bounds
            .map_origin(|value| ScaledPixels(round_half_toward_zero(value.0)))
            .map_size(|size| size.ceil());
        let masked_paint = self.masked_paint_stack.last().cloned();
        if let Some(MaskedPaint::Effect {
            bounds: effect_bounds,
            shader,
            uniforms,
            time,
            opacity,
        }) = masked_paint.as_ref()
        {
            self.next_frame.scene.insert_primitive(EffectQuad {
                order: 0,
                bounds: final_bounds,
                effect_bounds: self.snap_bounds(*effect_bounds),
                transformation,
                content_mask,
                shader: shader.clone(),
                uniforms: shader.mask_uniforms(*uniforms, color),
                time: *time,
                corner_radii: Corners::default(),
                opacity: element_opacity * *opacity,
                image_tile: Some(tile),
                second_image_tile: None,
                third_image_tile: None,
                fourth_image_tile: None,
            });
            return Ok(());
        }
        let (background, background_bounds) = match masked_paint {
            Some(MaskedPaint::Fill {
                background,
                bounds: background_bounds,
            }) => (
                background.opacity(element_opacity),
                self.snap_bounds(background_bounds),
            ),
            _ => (
                Background::from(color.opacity(element_opacity)),
                final_bounds,
            ),
        };

        self.next_frame.scene.insert_primitive(MonochromeSprite {
            order: 0,
            pad: 0,
            bounds: final_bounds,
            content_mask,
            background,
            background_bounds,
            tile,
            transformation,
        });

        Ok(())
    }

    /// Paints a colored SVG using [`ObjectFit::Contain`] into the scene for the next frame.
    /// Uncached images with supplied bytes, and large asset images, rasterize on
    /// the background executor and request a redraw when ready. Their first paint
    /// may be empty; failures are logged.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_color_svg(
        &mut self,
        bounds: Bounds<Pixels>,
        path: SharedString,
        data: Option<&[u8]>,
        transformation: TransformationMatrix,
        corner_radii: Corners<Pixels>,
        current_color: Option<Hsla>,
        fill_color: Option<Hsla>,
        text_color: Option<Hsla>,
        cx: &App,
    ) -> Result<()> {
        self.paint_color_svg_with_fit(
            bounds,
            path,
            data,
            transformation,
            corner_radii,
            current_color,
            fill_color,
            text_color,
            ObjectFit::Contain,
            cx,
        )
    }

    /// Paints a colored SVG fitted and centered within the element's rounded bounds.
    /// Rasterization and caching follow [`Self::paint_color_svg`].
    #[allow(clippy::too_many_arguments)]
    pub fn paint_color_svg_with_fit(
        &mut self,
        bounds: Bounds<Pixels>,
        path: SharedString,
        data: Option<&[u8]>,
        transformation: TransformationMatrix,
        corner_radii: Corners<Pixels>,
        current_color: Option<Hsla>,
        fill_color: Option<Hsla>,
        text_color: Option<Hsla>,
        object_fit: ObjectFit,
        cx: &App,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let element_opacity = self.element_opacity();
        let logical_size = bounds.size;
        if logical_size.width <= crate::px(0.) || logical_size.height <= crate::px(0.) {
            return Ok(());
        }
        let bounds = self.snap_bounds(bounds);
        let params = RenderColorSvgParams {
            path,
            size: bounds.size.map(|pixels| {
                DevicePixels::from((pixels.0 * SMOOTH_SVG_SCALE_FACTOR).ceil() as i32)
            }),
            logical_size,
            object_fit,
            current_color,
            fill_color,
            text_color,
        };
        // External files can contain expensive patterns or filters even at icon sizes.
        let asynchronous = data.is_some() || color_svg::rasterize_in_background(&params);
        let Some(tile) =
            self.sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    if asynchronous {
                        return Ok(None);
                    }
                    let Some((size, bytes)) = cx.svg_renderer.render_color_image(&params, data)?
                    else {
                        return Ok(None);
                    };
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
        else {
            if asynchronous {
                self.request_color_svg_raster(params, data, cx);
            }
            return Ok(());
        };

        self.next_frame.scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            bounds,
            clip_bounds: bounds,
            content_mask: self.snapped_content_mask(),
            corner_radii: corner_radii.scale(self.scale_factor()),
            tile,
            opacity: element_opacity,
            transformation,
        });
        Ok(())
    }

    /// Paint an image into the scene for the next frame at the current z-index.
    ///
    /// The image geometry is also used as its clip geometry. Use
    /// [`Self::paint_image_with_clip`] when those regions need to differ.
    pub fn paint_image(
        &mut self,
        bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
        data: Arc<RenderImage>,
        frame_index: usize,
        grayscale: bool,
    ) -> Result<()> {
        self.paint_image_with_clip(bounds, bounds, corner_radii, data, frame_index, grayscale)
    }

    /// Paint an image into the scene with independent texture and clip bounds.
    ///
    /// `image_bounds` controls the texture geometry and sampling. `clip_bounds`
    /// and `corner_radii` control the visible image region independently. This
    /// distinction allows an image fitted with [`ObjectFit::Cover`](crate::ObjectFit::Cover)
    /// to retain the rounded corners of its element bounds.
    /// This method will panic if the frame_index is not valid
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_image_with_clip(
        &mut self,
        image_bounds: Bounds<Pixels>,
        clip_bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
        data: Arc<RenderImage>,
        frame_index: usize,
        grayscale: bool,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let bounds = self.snap_bounds(image_bounds);
        let clip_bounds = self.snap_bounds(clip_bounds);
        let params = RenderImageParams {
            image_id: data.id,
            frame_index,
        };

        let key = if let Some(lifetime) = &data.atlas_lifetime {
            self.sprite_atlas
                .retain_image(&params, Arc::downgrade(lifetime));
            self.next_frame.scene.retain_image(lifetime.clone());
            crate::AtlasKey::TransientImage(params)
        } else {
            params.into()
        };

        let tile = self
            .sprite_atlas
            .get_or_insert_with(&key, &mut || {
                Ok(Some((
                    data.size(frame_index),
                    Cow::Borrowed(
                        data.as_bytes(frame_index)
                            .expect("It's the caller's job to pass a valid frame index"),
                    ),
                )))
            })?
            .expect("Callback above only returns Some");
        let mut content_mask = self.snapped_content_mask();
        content_mask.bounds = content_mask.bounds.intersect(&clip_bounds);
        let corner_radii = corner_radii.scale(self.scale_factor());
        let opacity = self.element_opacity();

        self.next_frame.scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: grayscale.into(),
            bounds,
            clip_bounds,
            content_mask,
            corner_radii,
            tile,
            opacity,
            transformation: TransformationMatrix::default(),
        });
        Ok(())
    }

    /// Paint a surface into the scene for the next frame at the current z-index.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_surface(
        &mut self,
        bounds: Bounds<Pixels>,
        clip_bounds: Bounds<Pixels>,
        corner_radii: Corners<Pixels>,
        source: crate::SurfaceSource,
    ) {
        use crate::PaintSurface;

        self.invalidator.debug_assert_paint();

        let bounds = self.snap_bounds(bounds);
        let clip_bounds = self.snap_bounds(clip_bounds);
        let mut content_mask = self.snapped_content_mask();
        content_mask.bounds = content_mask.bounds.intersect(&clip_bounds);
        self.next_frame.scene.insert_primitive(PaintSurface {
            order: 0,
            bounds,
            clip_bounds,
            content_mask,
            corner_radii: corner_radii.scale(self.scale_factor()),
            opacity: self.element_opacity(),
            source,
        });
    }

    /// Removes an image from the sprite atlas.
    pub fn drop_image(&mut self, data: Arc<RenderImage>) -> Result<()> {
        if let Some(animation) = data.animation() {
            for image_id in animation.frame_ids() {
                let params = RenderImageParams {
                    image_id: *image_id,
                    frame_index: 0,
                };
                self.sprite_atlas.remove(&params.clone().into());
                self.sprite_atlas
                    .remove(&crate::AtlasKey::TransientImage(params));
            }
            return Ok(());
        }
        for frame_index in 0..data.frame_count() {
            let params = RenderImageParams {
                image_id: data.id,
                frame_index,
            };

            self.sprite_atlas.remove(&params.clone().into());
            self.sprite_atlas
                .remove(&crate::AtlasKey::TransientImage(params));
        }

        Ok(())
    }
}
