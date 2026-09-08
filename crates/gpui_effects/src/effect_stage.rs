use gpui::{
    App, AtlasTile, EffectShader, EffectUniforms, ImageSource, Pixels, SubtreeEffectPass, Window,
    px,
};
use smallvec::SmallVec;

#[derive(Clone, Default)]
pub(crate) struct StageImages(pub SmallVec<[ImageSource; 3]>);

impl std::fmt::Debug for StageImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageImages")
            .field("count", &self.0.len())
            .finish()
    }
}

impl StageImages {
    pub(crate) fn preload(&self, window: &mut Window, cx: &mut App) {
        for source in &self.0 {
            let _ = source.use_data(None, window, cx);
        }
    }

    pub(crate) fn prepare(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<SmallVec<[AtlasTile; 3]>> {
        self.0
            .iter()
            .map(|source| {
                let image = source.use_data(None, window, cx)?.ok()?;
                window.prepare_effect_image(&image, 0).ok()
            })
            .collect()
    }
}

/// One configurable image-processing stage in a subtree effect chain.
#[derive(Clone, Debug)]
pub struct EffectStage {
    pub(crate) images: StageImages,
    pub(crate) shader: EffectShader,
    pub(crate) uniforms: EffectUniforms,
    pub(crate) pixel_uniform_slots: [bool; gpui::EFFECT_UNIFORM_SLOTS],
    pub(crate) padding: Pixels,
    pub(crate) enabled: bool,
    pub(crate) bloom: Option<gpui::SubtreeBloomPass>,
    pub(crate) feedback: Option<gpui::SubtreeFeedbackPass>,
    pub(crate) distance_field: Option<gpui::SubtreeDistanceFieldPass>,
    pub(crate) particles: Option<gpui::SubtreeParticlePass>,
    pub(crate) particle_transition: Option<gpui::SubtreeParticleTransitionPass>,
}

impl EffectStage {
    /// Creates a stage using a single-image shader.
    pub fn new(shader: EffectShader) -> Self {
        assert!(
            shader.image_count() == 1 && !shader.is_mask(),
            "effect stages require a single-image shader"
        );
        Self {
            images: StageImages::default(),
            shader,
            uniforms: EffectUniforms::default(),
            pixel_uniform_slots: [false; gpui::EFFECT_UNIFORM_SLOTS],
            padding: px(0.),
            enabled: true,
            bloom: None,
            feedback: None,
            distance_field: None,
            particles: None,
            particle_transition: None,
        }
    }

    /// Creates a stage with one or three external images after the captured source.
    /// Images use the first decoded frame. While an input is unavailable, the stage is skipped.
    pub fn with_images(
        shader: EffectShader,
        images: impl IntoIterator<Item = ImageSource>,
    ) -> Self {
        let images: SmallVec<[ImageSource; 3]> = images.into_iter().collect();
        assert!(
            matches!(shader.image_count(), 2 | 4)
                && !shader.is_mask()
                && images.len() + 1 == usize::from(shader.image_count()),
            "external images must match the shader inputs after the captured source"
        );
        let mut stage = Self::new(crate::subtree_identity_shader());
        stage.shader = shader;
        stage.images = StageImages(images);
        stage
    }

    /// Replaces all uniforms, removing logical-pixel conversion.
    pub fn uniforms(mut self, uniforms: EffectUniforms) -> Self {
        self.uniforms = uniforms;
        self.pixel_uniform_slots.fill(false);
        self
    }

    /// Sets a raw uniform slot.
    pub fn uniform(mut self, index: usize, value: [f32; 4]) -> Self {
        self.uniforms.set_slot(index, value);
        self.pixel_uniform_slots[index] = false;
        self
    }

    /// Sets a slot in logical pixels, converted using the current window scale.
    pub fn uniform_pixels(mut self, index: usize, value: [Pixels; 4]) -> Self {
        self.uniforms.set_slot(index, value.map(f32::from));
        self.pixel_uniform_slots[index] = true;
        self
    }

    /// Adds paint-only space required by this stage. Active stage padding is summed.
    pub fn capture_padding(mut self, padding: Pixels) -> Self {
        self.padding = padding.max(px(0.));
        self
    }

    /// Enables this stage. Disabled stages consume no rendering pass or padding.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(crate) fn prepare(&self, scale: f32, time: f32) -> SubtreeEffectPass {
        let mut uniforms = self.uniforms;
        for (index, scale_pixels) in self.pixel_uniform_slots.iter().enumerate() {
            if *scale_pixels {
                uniforms.set_slot(index, self.uniforms.slots()[index].map(|v| v * scale));
            }
        }
        SubtreeEffectPass {
            images: Default::default(),
            shader: self.shader.clone(),
            uniforms,
            time,
            bloom: self.bloom.clone(),
            distance_field: self.distance_field.clone(),
            particle_transition: self.particle_transition.map(|mut transition| {
                transition.scale_factor = scale;
                transition
            }),
            particles: self.particles.clone().map(|mut particles| {
                particles.scale_factor = scale;
                particles
            }),
            feedback: self.feedback.clone().map(|mut feedback| {
                feedback.scale_factor = scale;
                feedback
            }),
        }
    }
}
