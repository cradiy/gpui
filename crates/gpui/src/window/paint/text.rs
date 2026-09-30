use super::super::{MaskedPaint, Window};
use crate::util::{round_half_toward_zero, round_to_device_pixel};
use crate::{
    Background, Bounds, Corners, EffectQuad, FontId, GlyphId, Hsla, IsZero, MonochromeSprite,
    Pixels, Point, PolychromeSprite, RenderGlyphParams, SUBPIXEL_VARIANTS_X, SUBPIXEL_VARIANTS_Y,
    ScaledPixels, StrikethroughStyle, SubpixelSprite, TextRenderingMode, Transformation,
    TransformationMatrix, Underline, UnderlineStyle, WindowBackgroundAppearance, px, size,
};
use anyhow::Result;
use std::borrow::Cow;

impl Window {
    /// Paint an underline into the scene for the next frame at the current z-index.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_underline(
        &mut self,
        origin: Point<Pixels>,
        width: Pixels,
        style: &UnderlineStyle,
    ) {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.raster_scale_factor();
        let thickness = self.snap_stroke(style.thickness);
        let height = if style.wavy {
            ScaledPixels(thickness.0 * 3.)
        } else {
            thickness
        };
        let bounds = Bounds {
            origin: origin.map(|c| ScaledPixels(round_to_device_pixel(c.0, scale_factor))),
            size: size(self.snap_stroke(width), height),
        };
        let element_opacity = self.element_opacity();

        self.next_frame.scene.insert_primitive(Underline {
            order: 0,
            pad: 0,
            bounds,
            content_mask: self.snapped_content_mask(),
            color: style.color.unwrap_or_default().opacity(element_opacity),
            thickness,
            wavy: style.wavy.into(),
        });
    }

    /// Paint a strikethrough into the scene for the next frame at the current z-index.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_strikethrough(
        &mut self,
        origin: Point<Pixels>,
        width: Pixels,
        style: &StrikethroughStyle,
    ) {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.raster_scale_factor();
        let height = style.thickness;
        let bounds = Bounds {
            origin: origin.map(|c| ScaledPixels(round_to_device_pixel(c.0, scale_factor))),
            size: size(self.snap_stroke(width), self.snap_stroke(height)),
        };
        let opacity = self.element_opacity();

        self.next_frame.scene.insert_primitive(Underline {
            order: 0,
            pad: 0,
            bounds,
            content_mask: self.snapped_content_mask(),
            thickness: self.snap_stroke(style.thickness),
            color: style.color.unwrap_or_default().opacity(opacity),
            wavy: false.into(),
        });
    }

    /// Paints a monochrome (non-emoji) glyph into the scene for the next frame at the current z-index.
    ///
    /// The y component of the origin is the baseline of the glyph.
    /// You should generally prefer to use the [`ShapedLine::paint`](crate::ShapedLine::paint) or
    /// [`WrappedLine::paint`](crate::WrappedLine::paint) methods in the [`TextSystem`](crate::TextSystem).
    /// This method is only useful if you need to paint a single glyph that has already been shaped.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_glyph(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
    ) -> Result<()> {
        self.paint_glyph_with_transformation(
            origin,
            font_id,
            glyph_id,
            font_size,
            color,
            Transformation::default(),
            origin,
        )
    }

    /// Paints a shaped monochrome glyph with a GPU transformation.
    ///
    /// `anchor` is expressed in logical window pixels and can be shared by a
    /// group of glyphs. The transformation affects painting only; text layout
    /// and hit testing remain unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_glyph_with_transformation(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
        transformation: Transformation,
        anchor: Point<Pixels>,
    ) -> Result<()> {
        self.paint_glyph_with_transformation_and_blur(
            origin,
            font_id,
            glyph_id,
            font_size,
            color,
            transformation,
            anchor,
            px(0.),
        )
    }

    /// Paints a monochrome glyph with a cached Gaussian blur mask.
    ///
    /// The blur expands only painting bounds; text layout and hit testing are
    /// unchanged. Rasterized masks are cached in the sprite atlas by glyph,
    /// scale, subpixel position, and device-pixel blur radius.
    pub fn paint_blurred_glyph(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
        blur_radius: Pixels,
    ) -> Result<()> {
        self.paint_glyph_with_transformation_and_blur(
            origin,
            font_id,
            glyph_id,
            font_size,
            color,
            Transformation::default(),
            origin,
            blur_radius,
        )
    }

    /// Paints a transformed monochrome glyph with a cached Gaussian blur mask.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_blurred_glyph_with_transformation(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
        transformation: Transformation,
        anchor: Point<Pixels>,
        blur_radius: Pixels,
    ) -> Result<()> {
        self.paint_glyph_with_transformation_and_blur(
            origin,
            font_id,
            glyph_id,
            font_size,
            color,
            transformation,
            anchor,
            blur_radius,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_glyph_with_transformation_and_blur(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        color: Hsla,
        transformation: Transformation,
        anchor: Point<Pixels>,
        blur_radius: Pixels,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let element_opacity = self.element_opacity();
        let scale_factor = self.raster_scale_factor();
        let transformation = transformation.into_matrix(anchor, scale_factor);
        let glyph_origin = origin.scale(scale_factor);

        let quantized_origin = Point::new(
            round_half_toward_zero(glyph_origin.x.0 * SUBPIXEL_VARIANTS_X as f32)
                / SUBPIXEL_VARIANTS_X as f32,
            round_half_toward_zero(glyph_origin.y.0 * SUBPIXEL_VARIANTS_Y as f32)
                / SUBPIXEL_VARIANTS_Y as f32,
        );
        let subpixel_variant = Point::new(
            (quantized_origin.x.fract() * SUBPIXEL_VARIANTS_X as f32) as u8,
            (quantized_origin.y.fract() * SUBPIXEL_VARIANTS_Y as f32) as u8,
        );
        let integer_origin = quantized_origin.map(|c| ScaledPixels(c.trunc()));
        // Arbitrary per-pixel fills are incompatible with LCD subpixel text,
        // whose three coverage channels assume one constant foreground color.
        let blur_radius = ((blur_radius.max(px(0.)).0 * scale_factor).ceil() as u32)
            .min(u32::from(u16::MAX)) as u16;
        let subpixel_rendering = blur_radius == 0
            && transformation == TransformationMatrix::unit()
            && self.masked_paint_stack.is_empty()
            && self.should_use_subpixel_rendering(font_id, font_size);
        let dilation = self.text_system().glyph_dilation_for_color(color);
        let params = RenderGlyphParams {
            font_id,
            glyph_id,
            font_size,
            subpixel_variant,
            scale_factor,
            is_emoji: false,
            subpixel_rendering,
            dilation,
            blur_radius,
        };

        let raster_bounds = self.text_system().raster_bounds(&params)?;
        if !raster_bounds.is_zero() {
            let tile = self
                .sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    let (size, bytes) = self.text_system().rasterize_glyph(&params)?;
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
                .expect("Callback above only errors or returns Some");
            let bounds = Bounds {
                origin: integer_origin + raster_bounds.origin.map(Into::into),
                size: tile.bounds.size.map(Into::into),
            };
            let content_mask = self.snapped_content_mask();
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
                    bounds,
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
                _ => (Background::from(color.opacity(element_opacity)), bounds),
            };

            if subpixel_rendering {
                self.next_frame.scene.insert_primitive(SubpixelSprite {
                    order: 0,
                    pad: 0,
                    bounds,
                    content_mask,
                    background,
                    background_bounds,
                    tile,
                    transformation,
                });
            } else {
                self.next_frame.scene.insert_primitive(MonochromeSprite {
                    order: 0,
                    pad: 0,
                    bounds,
                    content_mask,
                    background,
                    background_bounds,
                    tile,
                    transformation,
                });
            }
        }
        Ok(())
    }

    fn should_use_subpixel_rendering(&self, font_id: FontId, font_size: Pixels) -> bool {
        if self.next_frame.scene.is_capturing_subtree() {
            return false;
        }
        if self.platform_window.background_appearance() != WindowBackgroundAppearance::Opaque {
            return false;
        }

        if !self.platform_window.is_subpixel_rendering_supported() {
            return false;
        }

        let mode = match self.text_rendering_mode.get() {
            TextRenderingMode::PlatformDefault => self
                .text_system()
                .recommended_rendering_mode(font_id, font_size),
            mode => mode,
        };

        mode == TextRenderingMode::Subpixel
    }

    /// Paints an emoji glyph into the scene for the next frame at the current z-index.
    ///
    /// The y component of the origin is the baseline of the glyph.
    /// You should generally prefer to use the [`ShapedLine::paint`](crate::ShapedLine::paint) or
    /// [`WrappedLine::paint`](crate::WrappedLine::paint) methods in the [`TextSystem`](crate::TextSystem).
    /// This method is only useful if you need to paint a single emoji that has already been shaped.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_emoji(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
    ) -> Result<()> {
        self.paint_emoji_with_transformation(
            origin,
            font_id,
            glyph_id,
            font_size,
            Transformation::default(),
            origin,
        )
    }

    /// Paints a shaped emoji glyph with a GPU transformation around `anchor`.
    pub fn paint_emoji_with_transformation(
        &mut self,
        origin: Point<Pixels>,
        font_id: FontId,
        glyph_id: GlyphId,
        font_size: Pixels,
        transformation: Transformation,
        anchor: Point<Pixels>,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        let scale_factor = self.raster_scale_factor();
        let transformation = transformation.into_matrix(anchor, scale_factor);
        let glyph_origin = origin.scale(scale_factor);
        let integer_origin = glyph_origin.map(|c| ScaledPixels(round_half_toward_zero(c.0)));
        let params = RenderGlyphParams {
            font_id,
            glyph_id,
            font_size,
            subpixel_variant: Default::default(),
            scale_factor,
            is_emoji: true,
            subpixel_rendering: false,
            dilation: 0,
            blur_radius: 0,
        };

        let raster_bounds = self.text_system().raster_bounds(&params)?;
        if !raster_bounds.is_zero() {
            let tile = self
                .sprite_atlas
                .get_or_insert_with(&params.clone().into(), &mut || {
                    let (size, bytes) = self.text_system().rasterize_glyph(&params)?;
                    Ok(Some((size, Cow::Owned(bytes))))
                })?
                .expect("Callback above only errors or returns Some");

            let bounds = Bounds {
                origin: integer_origin + raster_bounds.origin.map(Into::into),
                size: tile.bounds.size.map(Into::into),
            };
            let content_mask = self.snapped_content_mask();
            let opacity = self.element_opacity();

            self.next_frame.scene.insert_primitive(PolychromeSprite {
                order: 0,
                pad: 0,
                grayscale: false.into(),
                bounds,
                clip_bounds: bounds,
                corner_radii: Default::default(),
                content_mask,
                tile,
                opacity,
                transformation,
            });
        }
        Ok(())
    }
}
