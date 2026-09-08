use crate::{EffectStage, SubtreeEffect, subtree_effect_chain, subtree_identity_shader};
pub use gpui::ParticleTransitionOptions;
use gpui::{IntoElement, SubtreeParticleTransitionPass, px};

impl EffectStage {
    /// Fragments the source into particles along deterministic, reversible paths.
    /// Progress zero preserves the source; one is transparent. The caller owns
    /// the clock. Keep source, layout and options stable while reversing progress.
    pub fn particle_transition(progress: f32, options: ParticleTransitionOptions) -> Self {
        let options = options.normalized();
        let padding = f32::from(options.scatter.x)
            .abs()
            .max(f32::from(options.scatter.y).abs())
            + f32::from(options.spread) * 1.25
            + f32::from(options.streak)
            + f32::from(options.radius) * 3.
            + f32::from(options.cell_size)
            + 2.;
        let mut stage = Self::new(subtree_identity_shader()).capture_padding(px(padding));
        stage.particle_transition = Some(SubtreeParticleTransitionPass {
            progress: if progress.is_finite() {
                progress.clamp(0., 1.)
            } else {
                0.
            },
            options,
            scale_factor: 1.,
        });
        stage
    }
}

/// Dissolves painted content into particles, or gathers them by decreasing progress.
pub fn subtree_particle_transition<E: IntoElement>(
    element: E,
    progress: f32,
    options: ParticleTransitionOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(
        element,
        [EffectStage::particle_transition(progress, options)],
    )
}
