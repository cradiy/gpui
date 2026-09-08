use gpui::{EffectShader, EffectUniforms, Pixels, SubtreeEffectPass, px};

/// One configurable image-processing stage in a subtree effect chain.
#[derive(Clone, Debug)]
pub struct EffectStage {
    pub(crate) shader: EffectShader,
    pub(crate) uniforms: EffectUniforms,
    pub(crate) pixel_uniform_slots: [bool; gpui::EFFECT_UNIFORM_SLOTS],
    pub(crate) padding: Pixels,
    pub(crate) enabled: bool,
    pub(crate) bloom: Option<gpui::SubtreeBloomPass>,
    pub(crate) feedback: Option<gpui::SubtreeFeedbackPass>,
}

impl EffectStage {
    /// Creates a stage using a single-image shader.
    pub fn new(shader: EffectShader) -> Self {
        assert!(
            shader.image_count() == 1 && !shader.is_mask(),
            "effect stages require a single-image shader"
        );
        Self {
            shader,
            uniforms: EffectUniforms::default(),
            pixel_uniform_slots: [false; gpui::EFFECT_UNIFORM_SLOTS],
            padding: px(0.),
            enabled: true,
            bloom: None,
            feedback: None,
        }
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
            shader: self.shader.clone(),
            uniforms,
            time,
            bloom: self.bloom.clone(),
            feedback: self.feedback.clone().map(|mut feedback| {
                feedback.scale_factor = scale;
                feedback
            }),
        }
    }
}
