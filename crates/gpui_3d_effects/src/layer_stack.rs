use gpui_3d::Transform;

/// Centered poses for a stack of surfaces opening into spaced, gently fanned layers.
/// Sampling does not allocate, own a clock, or change geometry or material state.
///
/// ```
/// use gpui_3d::{Material, Mesh, Object, Scene};
/// use gpui_3d_effects::LayerStack;
///
/// let stack = LayerStack::new(5).spacing([0.6, 0.1, -0.3]);
/// let mesh = Mesh::plane();
/// let mut scene = Scene::new();
/// for pose in stack.sample(0.5) {
///     scene = scene.object(Object::new(mesh.clone(), Material::ui()).transform(pose));
/// }
/// ```
#[derive(Clone, Copy, Debug)]
pub struct LayerStack {
    layers: usize,
    spacing: [f32; 3],
    collapsed_spacing: [f32; 3],
    fan: [f32; 3],
}

impl LayerStack {
    /// Creates a stack ordered from front to back along negative Z.
    /// Zero layers yields no poses; one layer remains at the origin.
    pub fn new(layers: usize) -> Self {
        Self {
            layers,
            spacing: [0.6, 0.1, -0.3],
            collapsed_spacing: [0.025, 0.025, -0.055],
            fan: [0., 0.16, -0.12],
        }
    }

    /// Sets the displacement between adjacent fully expanded layers in scene units.
    /// Panics if a component or the resulting stack extent is not finite.
    #[track_caller]
    pub fn spacing(mut self, spacing: [f32; 3]) -> Self {
        self.validate_spacing(spacing);
        self.spacing = spacing;
        self
    }

    /// Sets the displacement between adjacent collapsed layers in scene units.
    /// Allow for the geometry's thickness to avoid intersections or coincident faces.
    /// Panics if a component or the resulting stack extent is not finite.
    #[track_caller]
    pub fn collapsed_spacing(mut self, spacing: [f32; 3]) -> Self {
        self.validate_spacing(spacing);
        self.collapsed_spacing = spacing;
        self
    }

    /// Sets the total XYZ Euler rotation span between the first and last open layers.
    /// Angles are in radians, centered around zero. Panics on non-finite angles.
    #[track_caller]
    pub fn fan(mut self, angles: [f32; 3]) -> Self {
        assert!(
            angles.iter().all(|v| v.is_finite()),
            "layer fan must be finite"
        );
        self.fan = angles;
        self
    }

    /// Samples linear expansion in [0, 1]. Apply easing to progress before sampling.
    /// Out-of-range progress is clamped; non-finite progress selects the closed pose.
    /// Poses retain unit scale and a stationary centroid at the origin.
    pub fn sample(self, progress: f32) -> impl ExactSizeIterator<Item = Transform> {
        let progress = if progress.is_finite() {
            progress.clamp(0., 1.)
        } else {
            0.
        };
        let last = self.layers.saturating_sub(1) as f64;
        let spacing: [f64; 3] = std::array::from_fn(|axis| {
            let closed = f64::from(self.collapsed_spacing[axis]);
            closed + (f64::from(self.spacing[axis]) - closed) * f64::from(progress)
        });
        (0..self.layers).map(move |index| {
            let offset = index as f64 - last * 0.5;
            let fraction = if last > 0. { offset / last } else { 0. };
            Transform {
                position: spacing.map(|step| (step * offset) as f32),
                rotation: self
                    .fan
                    .map(|span| (f64::from(span) * fraction * f64::from(progress)) as f32),
                ..Default::default()
            }
        })
    }

    #[track_caller]
    fn validate_spacing(&self, spacing: [f32; 3]) {
        let extent = self.layers.saturating_sub(1) as f64 * 0.5;
        assert!(
            spacing
                .iter()
                .all(|v| v.is_finite() && (f64::from(*v) * extent).abs() <= f64::from(f32::MAX)),
            "layer spacing must have a finite stack extent"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_and_fan_preserve_center_during_reversible_expansion() {
        let closed = [0.03, -0.02, -0.08];
        let open = [0.7, 0.15, -0.4];
        let fan = [0.08, -0.2, 0.12];
        for count in [0, 1, 4, 5] {
            let stack = LayerStack::new(count)
                .spacing(open)
                .collapsed_spacing(closed)
                .fan(fan);
            let forward: Vec<_> = (0..=20)
                .map(|step| stack.sample(step as f32 / 20.).collect::<Vec<_>>())
                .collect();
            for step in (0..=20).rev() {
                let progress = step as f32 / 20.;
                let poses: Vec<_> = stack.sample(progress).collect();
                assert_eq!(poses.len(), count);
                for (pose, original) in poses.iter().zip(&forward[step]) {
                    assert_eq!(pose.position, original.position);
                    assert_eq!(pose.rotation, original.rotation);
                    assert_eq!(pose.scale, [1.; 3]);
                }
                for axis in 0..3 {
                    assert!(
                        poses
                            .iter()
                            .map(|pose| pose.position[axis])
                            .sum::<f32>()
                            .abs()
                            < 1e-5
                    );
                    assert!(
                        poses
                            .iter()
                            .map(|pose| pose.rotation[axis])
                            .sum::<f32>()
                            .abs()
                            < 1e-5
                    );
                    for pair in poses.windows(2) {
                        let gap = closed[axis] + (open[axis] - closed[axis]) * progress;
                        assert!(
                            (pair[1].position[axis] - pair[0].position[axis] - gap).abs() < 1e-5
                        );
                    }
                    if count > 1 {
                        let span = poses[count - 1].rotation[axis] - poses[0].rotation[axis];
                        assert!((span - fan[axis] * progress).abs() < 1e-5);
                    }
                }
            }
        }
    }
}
