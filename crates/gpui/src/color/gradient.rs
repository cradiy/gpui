use super::*;
use std::collections::HashMap;

pub(super) fn next_gradient_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(super) struct ExtendedGradient {
    pub(super) stops: Vec<LinearColorStop>,
    pub(super) midpoints: Vec<f32>,
    #[serde(skip, default = "next_gradient_revision")]
    pub(super) revision: u64,
    #[serde(skip, default = "next_gradient_revision")]
    lineage: u64,
}

impl PartialEq for ExtendedGradient {
    fn eq(&self, other: &Self) -> bool {
        self.stops == other.stops && self.midpoints == other.midpoints
    }
}

impl ExtendedGradient {
    pub(super) fn new(stops: &[LinearColorStop]) -> Self {
        let revision = next_gradient_revision();
        Self {
            stops: stops.to_vec(),
            midpoints: vec![0.5; stops.len()],
            revision,
            lineage: revision,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub(super) struct ExtendedBorderGradient {
    stops: Vec<BorderColorStop>,
    #[serde(skip, default = "next_gradient_revision")]
    revision: u64,
    #[serde(skip, default = "next_gradient_revision")]
    lineage: u64,
}

impl PartialEq for ExtendedBorderGradient {
    fn eq(&self, other: &Self) -> bool {
        self.stops == other.stops
    }
}

impl ExtendedBorderGradient {
    pub(super) fn new(stops: &[BorderColorStop]) -> Self {
        let revision = next_gradient_revision();
        Self {
            stops: stops.to_vec(),
            revision,
            lineage: revision,
        }
    }
}

impl BorderGradient {
    /// Returns the stops in ascending perimeter position order.
    pub fn gradient_stops(&self) -> &[BorderColorStop] {
        self.extended
            .as_ref()
            .map_or(&self.stops[..(self.stop_count as usize).min(2)], |data| {
                &data.stops
            })
    }

    /// Replaces one stop without modifying other gradients cloned from this one.
    /// Positions must remain strictly increasing. Notify the owning view after
    /// editing so its cached drawing is refreshed.
    pub fn set_gradient_stop(&mut self, index: usize, stop: BorderColorStop) {
        let stops = self.gradient_stops();
        assert!(index < stops.len(), "gradient stop index out of range");
        assert!(
            (0.0..=1.0).contains(&stop.position),
            "border gradient stop position out of range"
        );
        assert!(
            index == 0 || stops[index - 1].position < stop.position,
            "border gradient stops must be strictly increasing"
        );
        assert!(
            index + 1 == stops.len() || stop.position < stops[index + 1].position,
            "border gradient stops must be strictly increasing"
        );
        if stops[index] == stop {
            return;
        }
        if let Some(data) = &mut self.extended {
            let data = Arc::make_mut(data);
            data.stops[index] = stop;
            data.revision = next_gradient_revision();
        }
        if index < 2 {
            self.stops[index] = stop;
        }
    }
}

/// Fixed-size perimeter gradient parameters consumed by the quad shader.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct GpuBorderGradient {
    pub(crate) stops: [BorderColorStop; 2],
    pub(crate) stop_count: u32,
    pub(crate) color_space: ColorSpace,
    pub(crate) phase: f32,
    pub(crate) opacity: f32,
    pub(crate) stop_offset: u32,
    pad: u32,
}

impl Default for GpuBorderGradient {
    fn default() -> Self {
        GradientBuffer::default().border(&BorderGradient::default())
    }
}

impl Background {
    /// Returns the midpoint of the segment starting at `index`, if it exists.
    pub fn gradient_midpoint_at(&self, index: usize) -> Option<f32> {
        if index >= (self.stop_count as usize).saturating_sub(1) {
            return None;
        }
        Some(self.extended.as_ref().map_or_else(
            || self.gradient_midpoints[index],
            |data| data.midpoints[index],
        ))
    }
    /// Returns the gradient stops in ascending position order.
    pub fn gradient_stops(&self) -> &[LinearColorStop] {
        self.extended
            .as_ref()
            .map_or(&self.colors[..(self.stop_count as usize).min(2)], |data| {
                &data.stops
            })
    }

    /// Replaces a stop without changing other backgrounds cloned from this one.
    /// The new position must remain between the adjacent stops. Notify the owning
    /// view after editing so its cached drawing is refreshed.
    pub fn set_gradient_stop(&mut self, index: usize, stop: LinearColorStop) {
        let stops = self.gradient_stops();
        assert!(index < stops.len(), "gradient stop index out of range");
        assert!(
            (0.0..=1.0).contains(&stop.percentage),
            "gradient stop position out of range"
        );
        assert!(
            index == 0 || stops[index - 1].percentage <= stop.percentage,
            "gradient stops must be ordered"
        );
        assert!(
            index + 1 == stops.len() || stop.percentage <= stops[index + 1].percentage,
            "gradient stops must be ordered"
        );
        if stops[index] == stop {
            return;
        }
        if let Some(data) = &mut self.extended {
            let data = Arc::make_mut(data);
            data.stops[index] = stop;
            data.revision = next_gradient_revision();
        }
        if index < 2 {
            self.colors[index] = stop;
        }
    }
}

/// Fixed-size background parameters consumed by the primitive shaders.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct GpuBackground {
    pub(crate) tag: BackgroundTag,
    pub(crate) color_space: ColorSpace,
    pub(crate) solid: Hsla,
    pub(crate) gradient_angle_or_pattern_height: f32,
    pub(crate) colors: [LinearColorStop; 2],
    pub(crate) stop_count: u32,
    pub(crate) gradient_phase: f32,
    pub(crate) gradient_repeating: u32,
    pub(crate) gradient_midpoints: [f32; 2],
    pub(crate) angular_seam_width: f32,
    pub(crate) stop_offset: u32,
}

impl Default for GpuBackground {
    fn default() -> Self {
        GradientBuffer::default().background(&Background::default())
    }
}

impl From<Hsla> for GpuBackground {
    fn from(color: Hsla) -> Self {
        GradientBuffer::default().background(&Background::from(color))
    }
}

impl From<Rgba> for GpuBackground {
    fn from(color: Rgba) -> Self {
        Hsla::from(color).into()
    }
}

/// A storage-buffer gradient stop, including the following segment's midpoint.
#[derive(Clone, Copy, Default, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct GpuGradientStop {
    /// Hue, saturation, lightness and alpha.
    pub color: [f32; 4],
    /// Position along the gradient.
    pub position: f32,
    /// Relative position of the segment's 50% color mixture.
    pub midpoint: f32,
}

/// Returns the smallest contiguous range of new stops that needs uploading.
pub fn gradient_changed_range(
    old: &[GpuGradientStop],
    new: &[GpuGradientStop],
) -> Option<std::ops::Range<usize>> {
    let start = new
        .iter()
        .enumerate()
        .position(|(index, stop)| old.get(index) != Some(stop))?;
    let end = new
        .iter()
        .enumerate()
        .rposition(|(index, stop)| old.get(index) != Some(stop))
        .unwrap()
        + 1;
    Some(start..end)
}

/// Scene-owned storage for long gradients. Slots are reused across scene rebuilds;
/// replay registers each immutable fill or border snapshot in the destination scene.
#[derive(Default)]
pub struct GradientBuffer {
    stops: Vec<GpuGradientStop>,
    slots: HashMap<u64, GradientSlot>,
    free: Vec<(usize, usize)>,
}

struct GradientSlot {
    offset: usize,
    capacity: usize,
    lineage: u64,
    used: bool,
}

impl GradientBuffer {
    pub(crate) fn clear(&mut self) {
        for slot in self.slots.values_mut() {
            slot.used = false;
        }
    }

    pub(crate) fn finish(&mut self) {
        self.slots.retain(|_, slot| {
            if !slot.used {
                self.free.push((slot.offset, slot.capacity));
            }
            slot.used
        });
        self.free.sort_unstable_by_key(|(offset, _)| *offset);
        let mut index = 0;
        while index + 1 < self.free.len() {
            if self.free[index].0 + self.free[index].1 == self.free[index + 1].0 {
                self.free[index].1 += self.free.remove(index + 1).1;
            } else {
                index += 1;
            }
        }
        if let Some(&(offset, capacity)) = self.free.last()
            && offset + capacity == self.stops.len()
        {
            self.stops.truncate(offset);
            self.free.pop();
        }
        if self.slots.is_empty() {
            self.stops.clear();
            self.free.clear();
        }
    }

    /// Returns packed stop data for this scene. Inline gradients need no entries.
    pub fn stops(&self) -> &[GpuGradientStop] {
        &self.stops
    }

    fn register(
        &mut self,
        revision: u64,
        lineage: u64,
        count: usize,
        mut stop_at: impl FnMut(usize) -> GpuGradientStop,
    ) -> u32 {
        if !self.slots.contains_key(&revision)
            && let Some(key) = self
                .slots
                .iter()
                .filter(|(_, slot)| !slot.used && slot.lineage == lineage && slot.capacity >= count)
                .min_by_key(|(_, slot)| slot.offset)
                .map(|(key, _)| *key)
        {
            let slot = self.slots.remove(&key).unwrap();
            self.free.push((slot.offset, slot.capacity));
        }
        let slot = self.slots.entry(revision).or_insert_with(|| {
            let available = self
                .free
                .iter()
                .enumerate()
                .filter(|(_, (_, capacity))| *capacity >= count)
                .min_by_key(|(_, (_, capacity))| *capacity)
                .map(|(index, _)| index);
            let (offset, capacity) = available
                .map(|index| self.free.swap_remove(index))
                .unwrap_or_else(|| {
                    let offset = self.stops.len();
                    let end = offset
                        .checked_add(count)
                        .expect("gradient buffer size overflow");
                    u32::try_from(end).expect("gradient buffer exceeds GPU address space");
                    self.stops.resize(end, GpuGradientStop::default());
                    (offset, count)
                });
            for index in 0..count {
                self.stops[offset + index] = stop_at(index);
            }
            GradientSlot {
                offset,
                capacity,
                lineage,
                used: false,
            }
        });
        slot.used = true;
        u32::try_from(slot.offset + 1).expect("gradient buffer exceeds GPU address space")
    }

    pub(crate) fn border(&mut self, gradient: &BorderGradient) -> GpuBorderGradient {
        let stop_offset = gradient.extended.as_ref().map_or(0, |data| {
            self.register(data.revision, data.lineage, data.stops.len(), |index| {
                let stop = data.stops[index];
                GpuGradientStop {
                    color: [stop.color.h, stop.color.s, stop.color.l, stop.color.a],
                    position: stop.position,
                    midpoint: 0.5,
                }
            })
        });
        GpuBorderGradient {
            stops: gradient.stops,
            stop_count: gradient.stop_count,
            color_space: gradient.color_space,
            phase: gradient.phase,
            opacity: gradient.opacity,
            stop_offset,
            pad: 0,
        }
    }

    pub(crate) fn background(&mut self, background: &Background) -> GpuBackground {
        let stop_offset = background.extended.as_ref().map_or(0, |data| {
            self.register(data.revision, data.lineage, data.stops.len(), |index| {
                let stop = data.stops[index];
                GpuGradientStop {
                    color: [stop.color.h, stop.color.s, stop.color.l, stop.color.a],
                    position: stop.percentage,
                    midpoint: data.midpoints[index],
                }
            })
        });
        GpuBackground {
            tag: background.tag,
            color_space: background.color_space,
            solid: background.solid,
            gradient_angle_or_pattern_height: background.gradient_angle_or_pattern_height,
            colors: background.colors,
            stop_count: background.stop_count,
            gradient_phase: background.gradient_phase,
            gradient_repeating: background.gradient_repeating,
            gradient_midpoints: background.gradient_midpoints,
            angular_seam_width: background.angular_seam_width,
            stop_offset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bounds, ContentMask, Quad, ScaledPixels, Scene, point, size};

    #[test]
    fn border_stops_share_storage_and_support_unbounded_counts() {
        for count in [2, 3, 4, 21, 256, 1024] {
            let stops: Vec<_> = (0..count)
                .map(|i| border_color_stop(rgb(i), i as f32 / count as f32))
                .collect();
            let gradient = border_gradient(&stops);
            let mut buffer = GradientBuffer::default();
            let gpu = buffer.border(&gradient);
            let faded = gradient.opacity(0.25).phase(1.75);
            let shared = buffer.border(&faded);
            assert_eq!(gpu.stop_offset, shared.stop_offset);
            assert_eq!(shared.opacity, 0.25);
            assert_eq!(shared.phase, 0.75);
            assert_eq!(gradient.gradient_stops(), stops);
            assert_eq!(
                buffer.stops().len(),
                if count == 2 { 0 } else { count as usize }
            );
            for (packed, stop) in buffer.stops().iter().zip(stops) {
                assert_eq!(
                    packed.color,
                    [stop.color.h, stop.color.s, stop.color.l, stop.color.a]
                );
                assert_eq!(packed.position, stop.position);
            }
            assert!(gradient.opacity(0.).is_transparent());
            assert!(gradient.opacity(-1.).is_transparent());
            assert_eq!(buffer.border(&gradient.opacity(2.)).opacity, 1.);
            assert!(!gradient.is_transparent());
        }
    }

    #[test]
    fn border_edits_preserve_snapshots_and_reuse_slots_alongside_fills() {
        let original = border_gradient(
            (0..32)
                .map(|i| border_color_stop(rgb(0xff0000), i as f32 / 32.))
                .collect::<Vec<_>>(),
        );
        let mut edited = original.clone();
        let fill = gradient();
        let mut buffer = GradientBuffer::default();
        let fill_offset = buffer.background(&fill).stop_offset;
        let border_offset = buffer.border(&edited).stop_offset;
        buffer.finish();
        for frame in 0..100 {
            let previous = buffer.stops().to_vec();
            edited.set_gradient_stop(15, border_color_stop(rgb(frame), 15. / 32.));
            buffer.clear();
            assert_eq!(buffer.background(&fill).stop_offset, fill_offset);
            assert_eq!(buffer.border(&edited).stop_offset, border_offset);
            buffer.finish();
            let index = border_offset as usize - 1 + 15;
            assert_eq!(
                gradient_changed_range(&previous, buffer.stops()),
                Some(index..index + 1)
            );
            assert_eq!(buffer.stops().len(), 52);
        }
        assert_eq!(original.gradient_stops()[15].color, rgb(0xff0000).into());
        buffer.clear();
        buffer.finish();
        assert!(buffer.stops().is_empty());
    }

    #[test]
    fn replay_registers_border_snapshots_in_the_destination_buffer() {
        let border = border_gradient(
            (0..32)
                .map(|i| border_color_stop(rgb(i), i as f32 / 32.))
                .collect::<Vec<_>>(),
        );
        let bounds = Bounds::new(
            point(ScaledPixels(0.), ScaledPixels(0.)),
            size(ScaledPixels(100.), ScaledPixels(100.)),
        );
        let mut source = Scene::default();
        source.insert_primitive(Quad {
            bounds,
            content_mask: ContentMask { bounds },
            border_gradient: border.clone(),
            ..Default::default()
        });
        source.finish();
        let mut replay = scene(gradient());
        replay.replay(0..source.len(), &source);
        replay.finish();
        drop(source);
        let gpu = &replay.quads[1].border_gradient;
        assert_eq!(gpu.stop_count, 32);
        let offset = gpu.stop_offset as usize - 1;
        assert_eq!(offset, 20);
        for (packed, stop) in replay.gradients.stops()[offset..offset + 32]
            .iter()
            .zip(border.gradient_stops())
        {
            assert_eq!(packed.position, stop.position);
            assert_eq!(
                packed.color,
                [stop.color.h, stop.color.s, stop.color.l, stop.color.a]
            );
        }
    }

    fn gradient() -> Background {
        multi_linear_gradient(
            90.,
            std::array::from_fn::<_, 20, _>(|i| linear_color_stop(rgb(0xff0000), i as f32 / 19.)),
        )
    }

    #[test]
    fn variable_stop_counts_share_and_reuse_storage() {
        let mut buffer = GradientBuffer::default();
        for count in [2, 3, 21, 256, 1024, 20, 3] {
            let stops: Vec<_> = (0..count)
                .map(|i| linear_color_stop(rgb(i as u32), i as f32 / (count - 1) as f32))
                .collect();
            let background = multi_linear_gradient(90., &stops);
            buffer.clear();
            let gpu = buffer.background(&background);
            let shared = buffer.background(&background.clone());
            buffer.finish();
            assert_eq!(gpu.stop_offset, shared.stop_offset);
            assert_eq!(background.gradient_stops(), stops);
            if count == 2 {
                assert!(buffer.stops().is_empty());
                assert_eq!(gpu.stop_offset, 0);
            } else {
                let offset = gpu.stop_offset as usize - 1;
                for (actual, expected) in buffer.stops()[offset..offset + count].iter().zip(&stops)
                {
                    assert_eq!(actual.position, expected.percentage);
                    assert_eq!(actual.color[0], expected.color.h);
                }
                assert!(buffer.stops().len() <= 2048);
            }
        }
    }

    fn scene(background: Background) -> Scene {
        let bounds = Bounds::new(
            point(ScaledPixels(0.), ScaledPixels(0.)),
            size(ScaledPixels(100.), ScaledPixels(100.)),
        );
        let mut scene = Scene::default();
        scene.insert_primitive(Quad {
            bounds,
            content_mask: ContentMask { bounds },
            background,
            ..Default::default()
        });
        scene.finish();
        scene
    }

    #[test]
    fn editing_shared_stops_preserves_snapshots_and_opacity_shares_storage() {
        let original = gradient();
        let mut edited = original.clone();
        edited.set_gradient_stop(10, linear_color_stop(rgb(0x0000ff), 10. / 19.));
        assert_eq!(original.gradient_stops()[10].color, rgb(0xff0000).into());
        assert_eq!(edited.gradient_stops()[10].color, rgb(0x0000ff).into());
        let faded = edited.opacity(0.5);
        assert!(Arc::ptr_eq(
            edited.extended.as_ref().unwrap(),
            faded.extended.as_ref().unwrap()
        ));
        assert_eq!(faded.solid.a, 0.5);
        assert!(edited.opacity(0.).is_transparent());
    }

    #[test]
    fn replay_retains_stops_after_source_is_destroyed() {
        let original = scene(gradient().gradient_midpoint(12, 0.2));
        let mut replay = Scene::default();
        replay.replay(0..original.len(), &original);
        replay.finish();
        assert_eq!(original.gradients.stops(), replay.gradients.stops());
        drop(original);
        assert_eq!(replay.gradients.stops()[12].midpoint, 0.2);
        let mut next = Scene::default();
        next.replay(0..replay.len(), &replay);
        next.finish();
        assert_eq!(next.gradients.stops(), replay.gradients.stops());
    }

    #[test]
    fn continuous_edits_reuse_slots_and_only_change_one_record() {
        let mut background = gradient();
        let mut buffer = GradientBuffer::default();
        buffer.background(&background);
        buffer.finish();
        for frame in 0..200 {
            let old = buffer.stops().to_vec();
            background.set_gradient_stop(10, linear_color_stop(rgb(frame), 10. / 19.));
            buffer.clear();
            buffer.background(&background);
            buffer.finish();
            assert_eq!(buffer.stops().len(), 20);
            assert_eq!(gradient_changed_range(&old, buffer.stops()), Some(10..11));
        }
        buffer.clear();
        buffer.finish();
        assert!(buffer.stops().is_empty());
    }

    #[test]
    fn editing_one_gradient_keeps_other_allocations_stable() {
        let mut backgrounds: Vec<_> = (0..32).map(|_| gradient()).collect();
        let mut buffer = GradientBuffer::default();
        let offsets: Vec<_> = backgrounds
            .iter()
            .map(|background| buffer.background(background).stop_offset)
            .collect();
        buffer.finish();
        for frame in 0..100 {
            let old = buffer.stops().to_vec();
            backgrounds[0].set_gradient_stop(10, linear_color_stop(rgb(frame), 10. / 19.));
            buffer.clear();
            let actual: Vec<_> = backgrounds
                .iter()
                .map(|background| buffer.background(background).stop_offset)
                .collect();
            buffer.finish();
            assert_eq!(actual, offsets);
            assert_eq!(gradient_changed_range(&old, buffer.stops()), Some(10..11));
        }
    }
}
