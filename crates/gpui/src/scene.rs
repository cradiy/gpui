// todo("windows"): remove
#![cfg_attr(windows, allow(dead_code))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, BackdropShader, Background, BorderGradient, Bounds, ContentMask,
    Corners, Edges, EffectShader, EffectUniforms, Hsla, Pixels, Point, Radians, ScaledPixels, Size,
    SurfaceSource, bounds_tree::BoundsTree, point,
};
use std::{
    fmt::Debug,
    iter::Peekable,
    ops::{Add, Range, Sub},
    slice,
    sync::Arc,
};

#[allow(non_camel_case_types, unused)]
#[expect(missing_docs)]
pub type PathVertex_ScaledPixels = PathVertex<ScaledPixels>;

#[expect(missing_docs)]
pub type DrawOrder = u32;

/// A boolean stored as a `u32` so that GPU-facing structs contain no
/// compiler-inserted padding bytes, which would be undefined behavior to
/// reinterpret as `&[u8]` when writing instance buffers. Guaranteed to be
/// `0` or `1` by construction; shaders read it as a `u32`/`uint`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PaddedBool32(u32);

impl From<bool> for PaddedBool32 {
    fn from(value: bool) -> Self {
        PaddedBool32(value as u32)
    }
}

#[derive(Default)]
#[expect(missing_docs)]
pub struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub backdrop_blurs: Vec<BackdropBlur>,
    pub shadows: Vec<Shadow>,
    pub quads: Vec<Quad>,
    pub effects: Vec<EffectQuad>,
    pub particles: Vec<crate::ParticleDraw>,
    pub subtree_layers: Vec<SubtreeLayer>,
    pending_subtrees: Vec<(EffectQuad, Arc<[SubtreeEffectPass]>, Scene)>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub subpixel_sprites: Vec<SubpixelSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub surfaces: Vec<PaintSurface>,
}

#[expect(missing_docs)]
impl Scene {
    pub fn clear(&mut self) {
        self.paint_operations.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.backdrop_blurs.clear();
        self.paths.clear();
        self.shadows.clear();
        self.quads.clear();
        self.effects.clear();
        self.particles.clear();
        self.subtree_layers.clear();
        self.pending_subtrees.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.subpixel_sprites.clear();
        self.polychrome_sprites.clear();
        self.surfaces.clear();
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        if let Some((_, _, scene)) = self.pending_subtrees.last_mut() {
            scene.push_layer(bounds);
            self.paint_operations
                .push(PaintOperation::StartLayer(bounds));
            return;
        }
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub fn pop_layer(&mut self) {
        if let Some((_, _, scene)) = self.pending_subtrees.last_mut() {
            scene.pop_layer();
            self.paint_operations.push(PaintOperation::EndLayer);
            return;
        }
        self.layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        if let Some((_, _, scene)) = self.pending_subtrees.last_mut() {
            scene.insert_primitive(primitive.clone());
            self.paint_operations
                .push(PaintOperation::Primitive(primitive));
            return;
        }
        let clipped_bounds = primitive
            .bounds()
            .intersect(&primitive.content_mask().bounds);

        if clipped_bounds.is_empty() {
            return;
        }

        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(clipped_bounds));
        match &mut primitive {
            Primitive::BackdropBlur(backdrop) => {
                backdrop.order = order;
                self.backdrop_blurs.push(backdrop.clone());
            }
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(*shadow);
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(*quad);
            }
            Primitive::Effect(effect) => {
                effect.order = order;
                self.effects.push(effect.clone());
            }
            Primitive::Particles(draw) => {
                draw.order = order;
                self.particles.push(draw.clone());
            }
            Primitive::SubtreeLayer(layer) => {
                layer.composite.order = order;
                self.subtree_layers.push(layer.clone());
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(*underline);
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(*sprite);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.order = order;
                self.subpixel_sprites.push(*sprite);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(*sprite);
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
        }
        self.paint_operations
            .push(PaintOperation::Primitive(primitive));
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => self.insert_primitive(primitive.clone()),
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
                PaintOperation::StartSubtree(composite, passes) => {
                    self.start_subtree_chain(composite.clone(), passes.clone())
                }
                PaintOperation::EndSubtree => self.end_subtree(),
            }
        }
    }

    pub fn finish(&mut self) {
        self.backdrop_blurs.sort_by_key(|backdrop| backdrop.order);
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.effects.sort_by_key(|effect| effect.order);
        self.particles.sort_by_key(|draw| draw.order);
        self.subtree_layers
            .sort_by_key(|layer| layer.composite.order);
        self.paths.sort_by_key(|path| path.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.subpixel_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.surfaces.sort_by_key(|surface| surface.order);
    }

    pub(crate) fn start_subtree(&mut self, composite: EffectQuad) {
        self.start_subtree_chain(composite, Arc::default());
    }

    pub(crate) fn start_subtree_chain(
        &mut self,
        composite: EffectQuad,
        passes: Arc<[SubtreeEffectPass]>,
    ) {
        self.paint_operations.push(PaintOperation::StartSubtree(
            composite.clone(),
            passes.clone(),
        ));
        self.pending_subtrees
            .push((composite, passes, Scene::default()));
    }

    pub(crate) fn end_subtree(&mut self) {
        let (composite, intermediate_effects, mut scene) = self
            .pending_subtrees
            .pop()
            .expect("unbalanced subtree capture");
        scene.finish();
        let layer = Primitive::SubtreeLayer(SubtreeLayer {
            composite,
            intermediate_effects,
            scene: Arc::new(scene),
        });
        if let Some((_, _, parent)) = self.pending_subtrees.last_mut() {
            parent.insert_primitive(layer);
        } else {
            let operation_count = self.paint_operations.len();
            self.insert_primitive(layer);
            self.paint_operations.truncate(operation_count);
        }
        self.paint_operations.push(PaintOperation::EndSubtree);
    }

    pub(crate) fn is_capturing_subtree(&self) -> bool {
        !self.pending_subtrees.is_empty()
    }

    /// Visits this scene and all captured child scenes in draw-tree order.
    pub fn visit(&self, visitor: &mut impl FnMut(&Scene)) {
        visitor(self);
        for layer in &self.subtree_layers {
            layer.scene.visit(visitor);
        }
    }

    /// Maximum nested subtree capture depth.
    pub fn subtree_depth(&self) -> usize {
        self.subtree_layers
            .iter()
            .map(|layer| 1 + layer.scene.subtree_depth())
            .max()
            .unwrap_or(0)
    }

    /// Peak render-target count for nested captures and ping-pong effect passes.
    pub fn subtree_target_count(&self) -> usize {
        self.subtree_layers
            .iter()
            .map(|layer| {
                (1 + layer.scene.subtree_target_count()).max(
                    if layer.intermediate_effects.is_empty() {
                        1
                    } else {
                        2
                    },
                )
            })
            .max()
            .unwrap_or(0)
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            backdrop_blurs_start: 0,
            backdrop_blurs_iter: self.backdrop_blurs.iter().peekable(),
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            effects_start: 0,
            effects_iter: self.effects.iter().peekable(),
            particles_start: 0,
            particles_iter: self.particles.iter().peekable(),
            subtree_layers_start: 0,
            subtree_layers_iter: self.subtree_layers.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            subpixel_sprites_start: 0,
            subpixel_sprites_iter: self.subpixel_sprites.iter().peekable(),
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    BackdropBlur,
    Shadow,
    #[default]
    Quad,
    Effect,
    Particles,
    SubtreeLayer,
    Path,
    Underline,
    MonochromeSprite,
    SubpixelSprite,
    PolychromeSprite,
    Surface,
}

pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
    StartSubtree(EffectQuad, Arc<[SubtreeEffectPass]>),
    EndSubtree,
}

#[derive(Clone)]
#[expect(missing_docs)]
pub enum Primitive {
    BackdropBlur(BackdropBlur),
    Shadow(Shadow),
    Quad(Quad),
    Effect(EffectQuad),
    Particles(crate::ParticleDraw),
    SubtreeLayer(SubtreeLayer),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintSurface),
}

#[expect(missing_docs)]
impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::BackdropBlur(backdrop) => &backdrop.bounds,
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Effect(effect) => &effect.bounds,
            Primitive::Particles(draw) => &draw.bounds,
            Primitive::SubtreeLayer(layer) => &layer.composite.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::SubpixelSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Surface(surface) => &surface.bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::BackdropBlur(backdrop) => &backdrop.content_mask,
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Effect(effect) => &effect.content_mask,
            Primitive::Particles(draw) => &draw.content_mask,
            Primitive::SubtreeLayer(layer) => &layer.composite.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
        }
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    backdrop_blurs_start: usize,
    backdrop_blurs_iter: Peekable<slice::Iter<'a, BackdropBlur>>,
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    effects_start: usize,
    effects_iter: Peekable<slice::Iter<'a, EffectQuad>>,
    particles_start: usize,
    particles_iter: Peekable<slice::Iter<'a, crate::ParticleDraw>>,
    subtree_layers_start: usize,
    subtree_layers_iter: Peekable<slice::Iter<'a, SubtreeLayer>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    subpixel_sprites_start: usize,
    subpixel_sprites_iter: Peekable<slice::Iter<'a, SubpixelSprite>>,
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.backdrop_blurs_iter.peek().map(|b| b.order),
                PrimitiveKind::BackdropBlur,
            ),
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (
                self.effects_iter.peek().map(|effect| effect.order),
                PrimitiveKind::Effect,
            ),
            (
                self.particles_iter.peek().map(|draw| draw.order),
                PrimitiveKind::Particles,
            ),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.subtree_layers_iter
                    .peek()
                    .map(|layer| layer.composite.order),
                PrimitiveKind::SubtreeLayer,
            ),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.monochrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                self.subpixel_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                self.polychrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Particles => {
                let start = self.particles_start;
                self.particles_iter.next();
                self.particles_start += 1;
                Some(PrimitiveBatch::Particles(start..start + 1))
            }
            PrimitiveKind::SubtreeLayer => {
                let start = self.subtree_layers_start;
                self.subtree_layers_iter.next();
                self.subtree_layers_start += 1;
                Some(PrimitiveBatch::SubtreeLayers(start..start + 1))
            }
            PrimitiveKind::BackdropBlur => {
                let backdrops_start = self.backdrop_blurs_start;
                let mut backdrops_end = backdrops_start + 1;
                self.backdrop_blurs_iter.next();
                while self
                    .backdrop_blurs_iter
                    .next_if(|backdrop| (backdrop.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    backdrops_end += 1;
                }
                self.backdrop_blurs_start = backdrops_end;
                Some(PrimitiveBatch::BackdropBlurs(
                    backdrops_start..backdrops_end,
                ))
            }
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| (shadow.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| (quad.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Effect => {
                let effects_start = self.effects_start;
                let mut effects_end = effects_start + 1;
                self.effects_iter.next();
                while self
                    .effects_iter
                    .next_if(|effect| (effect.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    effects_end += 1;
                }
                self.effects_start = effects_end;
                Some(PrimitiveBatch::Effects(effects_start..effects_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| (path.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| (underline.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::MonochromeSprite => {
                let texture_id = self.monochrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.monochrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.monochrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::SubpixelSprite => {
                let texture_id = self.subpixel_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.subpixel_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.subpixel_sprites_iter.next();
                while self
                    .subpixel_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.subpixel_sprites_start = sprites_end;
                Some(PrimitiveBatch::SubpixelSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::PolychromeSprite => {
                let texture_id = self.polychrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.polychrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.polychrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| (surface.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
#[allow(missing_docs)]
pub enum PrimitiveBatch {
    BackdropBlurs(Range<usize>),
    Shadows(Range<usize>),
    Quads(Range<usize>),
    Effects(Range<usize>),
    Particles(Range<usize>),
    SubtreeLayers(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    MonochromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    SubpixelSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    PolychromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    Surfaces(Range<usize>),
}

/// A background blur composited from primitives that precede it in scene draw order.
#[derive(Debug, Clone)]
#[expect(missing_docs)]
pub struct BackdropBlur {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub blur_radius: ScaledPixels,
    pub opacity: f32,
    pub shader: Option<BackdropShader>,
    pub uniforms: EffectUniforms,
    pub time: f32,
    pub pointer: Point<f32>,
    pub pointer_active: bool,
}

impl From<BackdropBlur> for Primitive {
    fn from(backdrop: BackdropBlur) -> Self {
        Primitive::BackdropBlur(backdrop)
    }
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_colors: Edges<Hsla>,
    pub border_gradient: BorderGradient,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

/// A captured child scene composited through an image effect.
#[derive(Clone)]
pub struct SubtreeLayer {
    /// Geometry and shader used to composite the captured texture.
    pub composite: EffectQuad,
    /// Ordered image passes applied before the final composite.
    pub intermediate_effects: Arc<[SubtreeEffectPass]>,
    /// Content drawn against transparent black before compositing.
    pub scene: Arc<Scene>,
}

/// An image-processing pass over an isolated subtree texture.
#[derive(Clone, Debug)]
pub struct SubtreeEffectPass {
    /// Single-image shader applied to the preceding texture.
    /// Compound stages use it to resolve their output.
    pub shader: EffectShader,
    /// Shader parameters, with pixel dimensions in device pixels.
    pub uniforms: EffectUniforms,
    /// Animation time in seconds.
    pub time: f32,
    /// Optional highlight extraction, separable blur and two-image composite.
    pub bloom: Option<SubtreeBloomPass>,
    /// Optional persistent two-image feedback pass. Mutually exclusive with bloom.
    pub feedback: Option<SubtreeFeedbackPass>,
}

/// A time-indexed update of a persistent feedback texture.
#[derive(Clone, Debug)]
pub struct SubtreeFeedbackPass {
    /// One identity per feedback surface in a scene.
    pub id: crate::EffectHistoryId,
    /// Two-image shader: current input followed by previous history.
    /// Slot 7 is reserved for `[retention, capture_gain, alpha_cutoff, 0]`.
    pub shader: EffectShader,
    /// Changing this value clears the retained pixels.
    pub generation: u64,
    /// Monotonic update number. Replaying a frame does not update history twice.
    pub frame: u64,
    /// Monotonic simulation time; leave unchanged while paused.
    pub time: std::time::Duration,
    /// Time for retained alpha to decay to 1/1024 of its original value.
    pub fade_duration: std::time::Duration,
    /// Whether this update adds the current input to history.
    pub capture: bool,
    /// Whether a visible element needs another animation frame.
    pub needs_animation: bool,
    /// Window scale factor. A change invalidates retained pixels.
    pub scale_factor: f32,
    /// History texture size divisor, clamped to 1 through 8.
    pub downsample: u32,
}

/// Shaders and target resolution for a bloom stage.
#[derive(Clone, Debug)]
pub struct SubtreeBloomPass {
    /// Single-image highlight extraction shader; slot 2.zw supplies the source-pixel footprint.
    pub extract: EffectShader,
    /// Single-image blur shader; slot 2.xy supplies the horizontal or vertical axis.
    pub blur: EffectShader,
    /// Two-image shader combining the stage input and blurred highlights.
    pub composite: EffectShader,
    /// Texture size divisor, clamped to 1 through 8 by the renderer.
    pub downsample: u32,
}

/// A custom fragment effect drawn over a rectangular region.
#[derive(Clone, Debug)]
pub struct EffectQuad {
    /// Scene draw order assigned during primitive insertion.
    pub order: DrawOrder,
    /// Device-scaled bounds covered by the effect.
    pub bounds: Bounds<ScaledPixels>,
    /// Device-scaled coordinate bounds exposed to the effect function.
    pub effect_bounds: Bounds<ScaledPixels>,
    /// GPU transformation applied without changing layout.
    pub transformation: TransformationMatrix,
    /// Device-scaled rectangular content mask.
    pub content_mask: ContentMask<ScaledPixels>,
    /// Shader used to evaluate each pixel.
    pub shader: EffectShader,
    /// User-defined effect parameters.
    pub uniforms: EffectUniforms,
    /// Animation time supplied to the effect function.
    pub time: f32,
    /// Device-scaled radii used to clip the effect.
    pub corner_radii: Corners<ScaledPixels>,
    /// Opacity applied after shader evaluation.
    pub opacity: f32,
    /// Optional atlas tile sampled by an image effect.
    pub image_tile: Option<AtlasTile>,
    /// Optional second atlas tile sampled by a two-image effect.
    pub second_image_tile: Option<AtlasTile>,
    /// Optional third atlas tile sampled by a four-image effect.
    pub third_image_tile: Option<AtlasTile>,
    /// Optional fourth atlas tile sampled by a four-image effect.
    pub fourth_image_tile: Option<AtlasTile>,
}

impl From<EffectQuad> for Primitive {
    fn from(effect: EffectQuad) -> Self {
        Primitive::Effect(effect)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Underline {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: PaddedBool32,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub element_bounds: Bounds<ScaledPixels>,
    pub element_corner_radii: Corners<ScaledPixels>,
    /// 0 = drop shadow (rendered outside the element), 1 = inset shadow (rendered inside).
    pub inset: u32,
    pub pad: u32, // align to 8 bytes
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    /// A solid border.
    #[default]
    Solid = 0,
    /// A dashed border.
    Dashed = 1,
}

/// A data type representing a 2 dimensional transformation that can be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale,
    /// stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    /// translation vector
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    /// The unit matrix, has no effect.
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    /// Move the origin by a given point
    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    /// Clockwise rotation in radians around the origin
    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    /// Scale around the origin
    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    /// Perform matrix multiplication with another transformation
    /// to produce a new transformation that is the result of
    /// applying both transformations: first, `other`, then `self`.
    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        // Perform matrix multiplication
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    /// Apply transformation to a point, mainly useful for debugging
    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub background_bounds: Bounds<ScaledPixels>,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct SubpixelSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub background_bounds: Bounds<ScaledPixels>,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<SubpixelSprite> for Primitive {
    fn from(sprite: SubpixelSprite) -> Self {
        Primitive::SubpixelSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub grayscale: PaddedBool32,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub clip_bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub clip_bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub opacity: f32,
    pub source: SurfaceSource,
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs)]
pub struct PathId(pub usize);

/// A line made up of a series of vertices and control points.
#[derive(Clone, Debug)]
#[expect(missing_docs)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub vertices: Vec<PathVertex<P>>,
    pub color: Background,
    start: Point<P>,
    current: Point<P>,
    contour_count: usize,
}

impl Path<Pixels> {
    /// Create a new path with the given starting point.
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            order: DrawOrder::default(),
            vertices: Vec::new(),
            start,
            current: start,
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
            contour_count: 0,
        }
    }

    /// Scale this path by the given factor.
    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            vertices: self
                .vertices
                .iter()
                .map(|vertex| vertex.scale(factor))
                .collect(),
            start: self.start.map(|start| start.scale(factor)),
            current: self.current.scale(factor),
            contour_count: self.contour_count,
            color: self.color,
        }
    }

    /// Move the start, current point to the given point.
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        self.start = to;
        self.current = to;
    }

    /// Draw a straight line from the current point to the given point.
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        self.current = to;
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        self.push_triangle(
            (self.current, ctrl, to),
            (point(0., 0.), point(0.5, 0.), point(1., 1.)),
        );
        self.current = to;
    }

    /// Push a triangle to the Path.
    pub fn push_triangle(
        &mut self,
        xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>),
        st: (Point<f32>, Point<f32>, Point<f32>),
    ) {
        self.bounds = self
            .bounds
            .union(&Bounds {
                origin: xy.0,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.1,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.2,
                size: Default::default(),
            });

        self.vertices.push(PathVertex {
            xy_position: xy.0,
            st_position: st.0,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.1,
            st_position: st.1,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.2,
            st_position: st.2,
            content_mask: Default::default(),
        });
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    #[expect(missing_docs)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subtree_composite() -> EffectQuad {
        EffectQuad {
            order: 0,
            bounds: test_bounds(),
            effect_bounds: test_bounds(),
            transformation: Default::default(),
            content_mask: ContentMask {
                bounds: test_bounds(),
            },
            shader: EffectShader::wgsl_image(
                "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
            ),
            uniforms: Default::default(),
            time: 0.,
            corner_radii: Default::default(),
            opacity: 1.,
            image_tile: None,
            second_image_tile: None,
            third_image_tile: None,
            fourth_image_tile: None,
        }
    }

    fn insert_test_quad(scene: &mut Scene) {
        scene.insert_primitive(Quad {
            bounds: test_bounds(),
            content_mask: ContentMask {
                bounds: test_bounds(),
            },
            ..Default::default()
        });
    }

    #[test]
    fn subtree_capture_preserves_nested_replay_and_sibling_order() {
        let mut scene = Scene::default();
        insert_test_quad(&mut scene);
        scene.start_subtree(subtree_composite());
        scene.push_layer(test_bounds());
        insert_test_quad(&mut scene);
        let child_start = scene.len();
        scene.start_subtree(subtree_composite());
        insert_test_quad(&mut scene);
        scene.end_subtree();
        let child_end = scene.len();
        scene.pop_layer();
        scene.end_subtree();
        insert_test_quad(&mut scene);
        scene.finish();

        let mut replayed = Scene::default();
        replayed.replay(0..scene.len(), &scene);
        replayed.finish();
        assert_eq!(replayed.quads.len(), 2);
        assert_eq!(replayed.subtree_layers.len(), 1);
        assert_eq!(replayed.subtree_depth(), 2);
        assert_eq!(replayed.subtree_layers[0].scene.quads.len(), 1);
        assert_eq!(
            replayed.subtree_layers[0].scene.subtree_layers[0]
                .scene
                .quads
                .len(),
            1
        );
        assert!(matches!(
            replayed.batches().collect::<Vec<_>>().as_slice(),
            [
                PrimitiveBatch::Quads(_),
                PrimitiveBatch::SubtreeLayers(_),
                PrimitiveBatch::Quads(_)
            ]
        ));

        let mut partial = Scene::default();
        partial.start_subtree(subtree_composite());
        partial.replay(child_start..child_end, &scene);
        partial.end_subtree();
        partial.finish();
        assert_eq!(partial.subtree_depth(), 2);
        assert_eq!(
            partial.subtree_layers[0].scene.subtree_layers[0]
                .scene
                .quads
                .len(),
            1
        );
    }

    #[test]
    fn empty_subtree_does_not_remove_previous_paint_operations() {
        let mut scene = Scene::default();
        insert_test_quad(&mut scene);
        let mut composite = subtree_composite();
        composite.bounds = Bounds::default();
        scene.start_subtree(composite);
        scene.end_subtree();
        let mut replayed = Scene::default();
        replayed.replay(0..scene.len(), &scene);
        assert_eq!(replayed.quads.len(), 1);
        assert!(!replayed.is_capturing_subtree());
    }

    #[test]
    fn effect_chain_replay_keeps_passes_without_recapturing_content() {
        let composite = subtree_composite();
        let pass = SubtreeEffectPass {
            shader: composite.shader.clone(),
            uniforms: EffectUniforms::new().with_slot(0, [0.2, 0.4, 0.6, 0.8]),
            time: 3.5,
            bloom: None,
            feedback: None,
        };
        let mut scene = Scene::default();
        scene.start_subtree_chain(composite, vec![pass.clone(); 7].into());
        insert_test_quad(&mut scene);
        scene.end_subtree();
        scene.finish();
        let mut replayed = Scene::default();
        replayed.replay(0..scene.len(), &scene);
        replayed.finish();
        assert_eq!(replayed.subtree_depth(), 1);
        assert_eq!(replayed.subtree_target_count(), 2);
        let layer = &replayed.subtree_layers[0];
        assert_eq!(layer.scene.quads.len(), 1);
        assert_eq!(layer.intermediate_effects.len(), 7);
        for effect in layer.intermediate_effects.iter() {
            assert_eq!(effect.shader.id(), pass.shader.id());
            assert_eq!(effect.uniforms, pass.uniforms);
            assert_eq!(effect.time, pass.time);
        }
    }

    fn test_bounds() -> Bounds<ScaledPixels> {
        Bounds {
            origin: Point {
                x: ScaledPixels(0.0),
                y: ScaledPixels(0.0),
            },
            size: Size {
                width: ScaledPixels(100.0),
                height: ScaledPixels(100.0),
            },
        }
    }

    #[test]
    fn backdrop_precedes_quads_in_the_same_layer() {
        let bounds = test_bounds();
        let mut scene = Scene::default();
        scene.push_layer(bounds);
        scene.insert_primitive(Quad {
            bounds,
            content_mask: ContentMask { bounds },
            ..Default::default()
        });
        scene.insert_primitive(BackdropBlur {
            order: 0,
            bounds,
            content_mask: ContentMask { bounds },
            corner_radii: Corners::default(),
            blur_radius: ScaledPixels(16.0),
            opacity: 1.0,
            shader: None,
            uniforms: EffectUniforms::default(),
            time: 0.0,
            pointer: Point { x: 0.5, y: 0.5 },
            pointer_active: false,
        });
        scene.pop_layer();
        scene.finish();

        let batches = scene.batches().collect::<Vec<_>>();
        assert!(matches!(
            batches.as_slice(),
            [PrimitiveBatch::BackdropBlurs(_), PrimitiveBatch::Quads(_)]
        ));
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PathVertex<P: Clone + Debug + Default + PartialEq> {
    pub xy_position: Point<P>,
    pub st_position: Point<f32>,
    pub content_mask: ContentMask<P>,
}

#[expect(missing_docs)]
impl PathVertex<Pixels> {
    pub fn scale(&self, factor: f32) -> PathVertex<ScaledPixels> {
        PathVertex {
            xy_position: self.xy_position.scale(factor),
            st_position: self.st_position,
            content_mask: self.content_mask.scale(factor),
        }
    }
}
