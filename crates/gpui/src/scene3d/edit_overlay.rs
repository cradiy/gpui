use crate::Rgba;

/// Appearance of fragments behind the group's independent occlusion surface.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum EditHiddenStyle3d {
    #[default]
    /// Discard occluded fragments.
    Hide,
    /// Draw occluded fragments with the hidden color.
    Solid,
    /// Dash and gap lengths in logical pixels, anchored at the segment's start.
    Dashed {
        /// Visible length of each repeated dash.
        dash: f32,
        /// Empty length between dashes.
        gap: f32,
    },
}

/// Screen-space appearance shared by a point or line. Colors are linear RGBA.
#[derive(Clone, Copy, Debug)]
pub struct EditStyle3d {
    /// Point diameter or line width in logical pixels.
    pub size: f32,
    /// Color of fragments in front of the occlusion source.
    pub color: Rgba,
    /// Color of occluded fragments when the hidden style is not Hide.
    pub hidden_color: Rgba,
    /// Appearance of occluded fragments. Dashed points use a solid marker.
    pub hidden: EditHiddenStyle3d,
    /// Nonnegative view-depth comparison tolerance; does not move geometry.
    pub depth_tolerance: f32,
}

impl EditStyle3d {
    /// Creates a style that hides occluded fragments.
    pub fn new(size: f32, color: Rgba) -> Self {
        Self {
            size,
            color,
            hidden_color: color,
            hidden: EditHiddenStyle3d::Hide,
            depth_tolerance: 0.0001,
        }
    }

    /// Checks finite colors, positive size, and valid dash dimensions.
    pub fn is_valid(&self) -> bool {
        self.size.is_finite()
            && self.size > 0.
            && self.size <= 4096.
            && self.depth_tolerance.is_finite()
            && self.depth_tolerance >= 0.
            && [self.color, self.hidden_color].iter().all(|c| {
                [c.r, c.g, c.b, c.a]
                    .iter()
                    .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            })
            && match self.hidden {
                EditHiddenStyle3d::Dashed { dash, gap } => {
                    dash.is_finite()
                        && gap.is_finite()
                        && dash > 0.
                        && gap > 0.
                        && (dash + gap).is_finite()
                }
                _ => true,
            }
    }
}

/// World-space point with a nonzero, group-local element identity.
#[derive(Clone, Copy, Debug)]
pub struct EditPoint3d {
    /// Application identity, unique across the group's points and lines.
    pub id: u32,
    /// Position in world coordinates.
    pub position: [f32; 3],
    /// Pixel diameter and fragment appearance.
    pub style: EditStyle3d,
}

/// World-space segment with a nonzero, group-local element identity.
#[derive(Clone, Copy, Debug)]
pub struct EditLine3d {
    /// Application identity, unique across the group's points and lines.
    pub id: u32,
    /// Start position in world coordinates and dash anchor.
    pub start: [f32; 3],
    /// End position in world coordinates.
    pub end: [f32; 3],
    /// Pixel width and fragment appearance.
    pub style: EditStyle3d,
}

impl super::OcclusionGroup3d {
    /// Whether element identities, coordinates, scaling and appearance are valid.
    pub fn elements_are_valid(&self) -> bool {
        if !self.pixel_scale.is_finite() || self.pixel_scale <= 0. {
            return false;
        }
        let mut ids = std::collections::HashSet::new();
        self.lines
            .iter()
            .map(|p| (p.id, p.start, p.end, p.style))
            .chain(
                self.points
                    .iter()
                    .map(|p| (p.id, p.position, p.position, p.style)),
            )
            .all(|(id, a, b, s)| {
                id != 0
                    && ids.insert(id)
                    && s.is_valid()
                    && a.iter().chain(&b).all(|v| v.is_finite())
                    && (s.size * self.pixel_scale).is_finite()
                    && s.size * self.pixel_scale > 0.
                    && s.size * self.pixel_scale <= 1_048_576.
                    && match s.hidden {
                        EditHiddenStyle3d::Dashed { dash, gap } => {
                            ((dash + gap) * self.pixel_scale).is_finite()
                                && dash * self.pixel_scale > 0.
                                && gap * self.pixel_scale > 0.
                        }
                        _ => true,
                    }
            })
    }
}
