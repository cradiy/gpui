use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui::{AtlasTile, EffectQuad, MeshTexture3d, Scene, Scene3dFrame, SubtreeEffectPass};

use super::{BackdropInstance, EffectInstance};

pub(super) struct SceneSnapshot {
    scene: Rc<Scene>,
    generations: Vec<u64>,
}

impl SceneSnapshot {
    pub(super) fn new(
        scene: &Rc<Scene>,
        mut generation: impl FnMut(AtlasTile) -> Option<u64>,
    ) -> Option<Self> {
        let mut tiles = Vec::new();
        if !scene_tiles(scene, &mut tiles) {
            return None;
        }
        Some(Self {
            scene: scene.clone(),
            generations: tiles
                .into_iter()
                .map(&mut generation)
                .collect::<Option<_>>()?,
        })
    }

    pub(super) fn matches(&self, other: &Self) -> bool {
        self.generations == other.generations && same_scene(&self.scene, &other.scene)
    }
}

fn same_scene(a: &Rc<Scene>, b: &Rc<Scene>) -> bool {
    let mut pending = vec![(a, b)];
    while let Some((a, b)) = pending.pop() {
        if Rc::ptr_eq(a, b) {
            continue;
        }
        if a.quads != b.quads
            || a.shadows != b.shadows
            || a.underlines != b.underlines
            || a.monochrome_sprites != b.monochrome_sprites
            || a.subpixel_sprites != b.subpixel_sprites
            || a.polychrome_sprites != b.polychrome_sprites
            || !same_items(&a.paths, &b.paths, |a, b| {
                a.order == b.order
                    && a.bounds == b.bounds
                    && a.content_mask == b.content_mask
                    && a.vertices == b.vertices
                    && a.color == b.color
            })
            || !same_items(&a.effects, &b.effects, same_effect)
            || !same_items(&a.backdrop_blurs, &b.backdrop_blurs, |a, b| {
                a.order == b.order
                    && a.shader.as_ref().map(|shader| shader.id())
                        == b.shader.as_ref().map(|shader| shader.id())
                    && bytemuck::bytes_of(&BackdropInstance::from(a))
                        == bytemuck::bytes_of(&BackdropInstance::from(b))
            })
            || a.subtree_layers.len() != b.subtree_layers.len()
        {
            return false;
        }
        for (a, b) in a.subtree_layers.iter().zip(&b.subtree_layers) {
            if !same_effect(&a.composite, &b.composite)
                || !same_passes(&a.intermediate_effects, &b.intermediate_effects)
                || !same_option(a.scene3d.as_ref(), b.scene3d.as_ref(), Arc::ptr_eq)
            {
                return false;
            }
            pending.push((&a.scene, &b.scene));
            match (&a.second_scene, &b.second_scene) {
                (Some(a), Some(b)) => pending.push((a, b)),
                (None, None) => {}
                _ => return false,
            }
        }
    }
    true
}

fn same_items<T>(a: &[T], b: &[T], mut same: impl FnMut(&T, &T) -> bool) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
}

fn same_option<T>(a: Option<&T>, b: Option<&T>, same: impl FnOnce(&T, &T) -> bool) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn same_effect(a: &EffectQuad, b: &EffectQuad) -> bool {
    a.order == b.order
        && a.shader.id() == b.shader.id()
        && a.image_tile == b.image_tile
        && a.second_image_tile == b.second_image_tile
        && a.third_image_tile == b.third_image_tile
        && a.fourth_image_tile == b.fourth_image_tile
        && bytemuck::bytes_of(&EffectInstance::from(a))
            == bytemuck::bytes_of(&EffectInstance::from(b))
}

pub(super) fn same_passes(a: &[SubtreeEffectPass], b: &[SubtreeEffectPass]) -> bool {
    same_items(a, b, |a, b| {
        a.shader.id() == b.shader.id()
            && a.uniforms == b.uniforms
            && a.time == b.time
            && a.images == b.images
            && a.feedback.is_none()
            && b.feedback.is_none()
            && a.particles.is_none()
            && b.particles.is_none()
            && a.particle_transition.is_none()
            && b.particle_transition.is_none()
            && same_option(a.bloom.as_ref(), b.bloom.as_ref(), |a, b| {
                a.extract.id() == b.extract.id()
                    && a.blur.id() == b.blur.id()
                    && a.composite.id() == b.composite.id()
                    && a.downsample == b.downsample
            })
            && same_option(
                a.distance_field.as_ref(),
                b.distance_field.as_ref(),
                |a, b| a.threshold == b.threshold && a.composite.id() == b.composite.id(),
            )
    })
}

pub(super) fn frame_tiles(frame: &Scene3dFrame, tiles: &mut Vec<AtlasTile>) {
    for object in frame.objects.iter() {
        if let MeshTexture3d::Image(tile) = object.texture {
            tiles.push(tile);
        }
        tiles.extend(
            [
                object.metallic_roughness_texture,
                object.emissive_texture,
                object.normal_texture,
                object.occlusion_texture,
            ]
            .into_iter()
            .flatten()
            .map(|texture| texture.tile),
        );
    }
}

fn effect_tiles(effect: &EffectQuad, tiles: &mut Vec<AtlasTile>) {
    tiles.extend(
        [
            effect.image_tile,
            effect.second_image_tile,
            effect.third_image_tile,
            effect.fourth_image_tile,
        ]
        .into_iter()
        .flatten(),
    );
}

pub(super) fn pass_tiles(passes: &[SubtreeEffectPass], tiles: &mut Vec<AtlasTile>) -> bool {
    for pass in passes {
        if pass.feedback.is_some() || pass.particles.is_some() || pass.particle_transition.is_some()
        {
            return false;
        }
        tiles.extend(pass.images.iter().copied());
    }
    true
}

fn scene_tiles(scene: &Scene, tiles: &mut Vec<AtlasTile>) -> bool {
    let mut reusable = true;
    scene.visit(&mut |scene| {
        reusable &=
            scene.particles.is_empty() && scene.fluids.is_empty() && scene.surfaces.is_empty();
        tiles.extend(scene.monochrome_sprites.iter().map(|sprite| sprite.tile));
        tiles.extend(scene.subpixel_sprites.iter().map(|sprite| sprite.tile));
        tiles.extend(scene.polychrome_sprites.iter().map(|sprite| sprite.tile));
        for effect in &scene.effects {
            effect_tiles(effect, tiles);
        }
        for layer in &scene.subtree_layers {
            effect_tiles(&layer.composite, tiles);
            reusable &= pass_tiles(&layer.intermediate_effects, tiles);
            if let Some(frame) = &layer.scene3d {
                frame_tiles(frame, tiles);
            }
        }
    });
    reusable
}

#[derive(Default)]
pub(super) struct OutputValidity {
    submitted: Cell<bool>,
    encoded: Cell<bool>,
}

impl OutputValidity {
    pub(super) fn reusable(&self) -> bool {
        self.submitted.get()
    }
    pub(super) fn encoded(&self) {
        self.encoded.set(true);
    }
    pub(super) fn commit(&self, submitted: bool) {
        if self.encoded.replace(false) {
            self.submitted.set(submitted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, ContentMask, EffectShader, Quad, ScaledPixels, point, rgba, size};

    fn bounds() -> Bounds<ScaledPixels> {
        Bounds::new(
            point(ScaledPixels(2.), ScaledPixels(3.)),
            size(ScaledPixels(48.), ScaledPixels(24.)),
        )
    }

    fn quad() -> Quad {
        Quad {
            bounds: bounds(),
            content_mask: ContentMask { bounds: bounds() },
            background: rgba(0x123456ff).into(),
            ..Default::default()
        }
    }

    fn tile() -> AtlasTile {
        AtlasTile {
            texture_id: gpui::AtlasTextureId {
                index: 0,
                kind: gpui::AtlasTextureKind::Monochrome,
            },
            tile_id: gpui::TileId(1),
            padding: 0,
            bounds: Bounds::new(
                point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
                size(gpui::DevicePixels(8), gpui::DevicePixels(8)),
            ),
        }
    }

    fn effect() -> EffectQuad {
        EffectQuad {
            order: 1,
            bounds: bounds(),
            effect_bounds: bounds(),
            transformation: Default::default(),
            content_mask: ContentMask { bounds: bounds() },
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

    fn snapshot(scene: Scene) -> SceneSnapshot {
        SceneSnapshot::new(&Rc::new(scene), |_| Some(1)).unwrap()
    }

    #[test]
    fn scene3d_capture_repaint_matches_pixels_but_not_style_geometry_or_order_changes() {
        let build = |quad: Quad| {
            let mut scene = Scene::default();
            scene.insert_primitive(quad);
            scene.finish();
            scene
        };
        let saved = snapshot(build(quad()));
        assert!(saved.matches(&snapshot(build(quad()))));
        for change in [
            |q: &mut Quad| q.background = rgba(0xff0000ff).into(),
            |q: &mut Quad| q.bounds.origin.x.0 += 1.,
            |q: &mut Quad| q.content_mask.bounds.size.width.0 -= 1.,
            |q: &mut Quad| q.corner_radii.top_left.0 = 8.,
            |q: &mut Quad| q.border_widths.top.0 = 2.,
        ] {
            let mut q = quad();
            change(&mut q);
            assert!(!saved.matches(&snapshot(build(q))));
        }
        let mut changed = build(quad());
        changed.quads[0].order += 1;
        assert!(!saved.matches(&snapshot(changed)));
    }

    #[test]
    fn scene3d_capture_keeps_glyph_atlas_versions_and_path_geometry_in_its_key() {
        let build = || {
            let mut scene = Scene::default();
            scene.insert_primitive(gpui::MonochromeSprite {
                order: 0,
                pad: 0,
                bounds: bounds(),
                content_mask: ContentMask { bounds: bounds() },
                background: rgba(0xabcdef80).into(),
                background_bounds: bounds(),
                tile: tile(),
                transformation: Default::default(),
            });
            let mut path = gpui::Path::new(point(gpui::px(3.), gpui::px(4.)));
            path.line_to(point(gpui::px(10.), gpui::px(18.)));
            path.line_to(point(gpui::px(22.), gpui::px(8.)));
            let mut path = path.scale(1.);
            path.content_mask = ContentMask { bounds: bounds() };
            path.color = rgba(0xff8040ff).into();
            scene.insert_primitive(path);
            scene.finish();
            scene
        };
        let original = Rc::new(build());
        let saved = SceneSnapshot::new(&original, |_| Some(1)).unwrap();
        assert!(saved.matches(&snapshot(build())));
        assert!(!saved.matches(&SceneSnapshot::new(&original, |_| Some(2)).unwrap()));
        assert!(SceneSnapshot::new(&original, |_| None).is_none());
        let mut changed = build();
        changed.paths[0].id = gpui::PathId(99);
        assert!(saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.paths[0].vertices[0].xy_position.x.0 += 2.;
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.monochrome_sprites[0].background_bounds.origin.x.0 += 3.;
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.monochrome_sprites[0].tile.texture_id.index += 1;
        assert!(!saved.matches(&snapshot(changed)));
    }

    #[test]
    fn scene3d_capture_compares_nested_effects_passes_and_backdrop_inputs() {
        let effect = effect();
        let build = || {
            let mut child = Scene::default();
            child.insert_primitive(quad());
            child.finish();
            let mut scene = Scene::default();
            scene.insert_primitive(gpui::Primitive::SubtreeLayer(gpui::SubtreeLayer {
                scene3d: None,
                scene: Rc::new(child),
                second_scene: None,
                composite: effect.clone(),
                intermediate_effects: vec![SubtreeEffectPass {
                    shader: effect.shader.clone(),
                    uniforms: Default::default(),
                    time: 0.,
                    images: Default::default(),
                    bloom: None,
                    feedback: None,
                    distance_field: Some(gpui::SubtreeDistanceFieldPass {
                        threshold: 0.5,
                        composite: effect.shader.clone(),
                    }),
                    particles: None,
                    particle_transition: None,
                }]
                .into(),
            }));
            scene.insert_primitive(gpui::BackdropBlur {
                order: 0,
                bounds: bounds(),
                content_mask: ContentMask { bounds: bounds() },
                corner_radii: Default::default(),
                blur_radius: ScaledPixels(4.),
                opacity: 1.,
                shader: None,
                uniforms: Default::default(),
                time: 0.,
                pointer: point(0.5, 0.5),
                pointer_active: false,
            });
            scene.finish();
            scene
        };
        let saved = snapshot(build());
        assert!(saved.matches(&snapshot(build())));
        let mut changed = build();
        let mut child = Scene::default();
        let mut changed_quad = quad();
        changed_quad.background = rgba(0x00ffffff).into();
        child.insert_primitive(changed_quad);
        child.finish();
        changed.subtree_layers[0].scene = Rc::new(child);
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.subtree_layers[0].composite.time = 0.1;
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.subtree_layers[0]
            .composite
            .uniforms
            .set_slot(0, [0.1; 4]);
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        let passes = Arc::make_mut(&mut changed.subtree_layers[0].intermediate_effects);
        passes[0].distance_field.as_mut().unwrap().threshold = 0.6;
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.subtree_layers[0].second_scene = Some(Rc::new(Scene::default()));
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.backdrop_blurs[0].pointer_active = true;
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        changed.subtree_layers[0].composite.shader = EffectShader::wgsl_image(
            "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv) * 0.5; }",
        );
        assert!(!saved.matches(&snapshot(changed)));
        let mut changed = build();
        Arc::make_mut(&mut changed.subtree_layers[0].intermediate_effects)[0].feedback =
            Some(gpui::SubtreeFeedbackPass {
                id: gpui::EffectHistoryId::new(),
                shader: effect.shader,
                generation: 0,
                frame: 0,
                time: Default::default(),
                fade_duration: std::time::Duration::from_secs(1),
                capture: true,
                needs_animation: true,
                scale_factor: 1.,
                downsample: 1,
            });
        assert!(SceneSnapshot::new(&Rc::new(changed), |_| Some(1)).is_none());
    }
}
