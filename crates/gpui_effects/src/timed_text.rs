use std::{cell::RefCell, collections::BTreeSet, ops::Range, rc::Rc, time::Duration};

use gpui::{
    App, Background, Bounds, ContentMask, EffectShader, EffectUniforms, Element, ElementId,
    GlobalElementId, GlyphRunTransform, Hitbox, InspectorElementId, InteractiveElement,
    Interactivity, IntoElement, LayoutId, Pixels, Point, Rgba, ShapedLine, SharedString,
    StyleRefinement, Styled, TextAlign, Transformation, Window, point, px, size, white,
};

/// Timing for one independently revealed text unit, usually a grapheme or word.
///
/// `range` is a UTF-8 byte range into the complete line. Units may have
/// different durations and gaps between them. Units with the same `group` are
/// emphasized together, which is useful when characters should reveal one by
/// one while their containing word lifts as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedTextUnit {
    /// UTF-8 byte range revealed during this unit.
    pub range: Range<usize>,
    /// Playback time at which revealing starts.
    pub start: Duration,
    /// Playback time at which revealing finishes.
    pub end: Duration,
    /// Identifier of the word or phrase emphasized with this unit.
    pub group: usize,
}

impl TimedTextUnit {
    /// Creates a timed unit. By default every unit is its own emphasis group.
    pub fn new(range: Range<usize>, start: Duration, end: Duration) -> Self {
        let group = range.start;
        Self {
            range,
            start,
            end,
            group,
        }
    }

    /// Makes this unit share an emphasis group with other units.
    pub fn group(mut self, group: usize) -> Self {
        self.group = group;
        self
    }
}

/// Paint-only emphasis applied to the currently active group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedTextEmphasis {
    /// Peak scale. `1.0` disables scaling.
    pub scale: f32,
    /// Peak translation in logical pixels. A negative Y value lifts the text.
    pub translation: Point<Pixels>,
    /// Extra distance pushed into each side of the active group.
    ///
    /// The renderer also accounts for the width added by scaling, so this is
    /// the additional elastic breathing room beyond avoiding overlap.
    pub surrounding_spread: Pixels,
    /// Fraction of group duration used to ease into the effect.
    pub enter_fraction: f32,
    /// Fraction of group duration used to ease out of the effect.
    pub exit_fraction: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TimedTextMotion {
    Elastic(TimedTextEmphasis),
    ProgressiveLift(Pixels),
    None,
}

impl Default for TimedTextEmphasis {
    fn default() -> Self {
        Self {
            scale: 1.08,
            translation: point(px(0.), px(-2.)),
            surrounding_spread: px(4.),
            enter_fraction: 0.18,
            exit_fraction: 0.22,
        }
    }
}

/// Soft leading opacity that travels ahead of the completed lyric fill.
///
/// The leading band fades into unreached text at its front edge and into the
/// completed fill at its back edge, avoiding a hard vertical reveal boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimedTextRevealWave {
    /// Distance between the completed fill and the front of the reveal.
    pub width: Pixels,
    /// Opacity of the active fill at the leading edge.
    pub leading_opacity: f32,
    /// Feathering applied to both edges of the leading band.
    pub softness: Pixels,
}

impl Default for TimedTextRevealWave {
    fn default() -> Self {
        Self {
            width: px(12.),
            leading_opacity: 0.16,
            softness: px(6.),
        }
    }
}

#[doc(hidden)]
#[derive(Clone, Default)]
pub struct TimedTextLayout(Rc<RefCell<Option<TimedTextLayoutInner>>>);

struct TimedTextLayoutInner {
    line: ShapedLine,
    line_height: Pixels,
    content_bounds: Bounds<Pixels>,
}

/// A single-line, single-element text renderer for karaoke and timed captions.
///
/// The line is shaped once. Arbitrarily timed UTF-8 ranges control a clipped
/// active fill, while the active word or phrase is transformed at glyph-paint
/// time around one shared anchor. The transformation does not alter layout or
/// move neighboring text.
pub struct TimedText {
    text: SharedString,
    units: Vec<TimedTextUnit>,
    position: Duration,
    active_fill: Background,
    inactive_opacity: f32,
    reveal_wave: Option<TimedTextRevealWave>,
    motion: TimedTextMotion,
    elastic_groups: Option<BTreeSet<usize>>,
    interactivity: Interactivity,
}

impl TimedText {
    /// Creates a timed single line.
    #[track_caller]
    pub fn new(
        text: impl Into<SharedString>,
        units: impl IntoIterator<Item = TimedTextUnit>,
    ) -> Self {
        let text = text.into();
        let mut units = units.into_iter().collect::<Vec<_>>();
        units.sort_by_key(|unit| (unit.start, unit.end));
        validate_units(&text, &units);

        Self {
            text,
            units,
            position: Duration::ZERO,
            active_fill: white().into(),
            inactive_opacity: 0.34,
            reveal_wave: Some(TimedTextRevealWave::default()),
            motion: TimedTextMotion::None,
            elastic_groups: None,
            interactivity: Interactivity::new(),
        }
    }

    /// Sets the playback position used for this frame.
    pub fn position(mut self, position: Duration) -> Self {
        self.position = position;
        self
    }

    /// Sets the solid or gradient fill for text that has been reached.
    pub fn active_fill(mut self, fill: impl Into<Background>) -> Self {
        self.active_fill = fill.into();
        self
    }

    /// Sets the opacity of unreached text relative to the inherited text color.
    pub fn inactive_opacity(mut self, opacity: f32) -> Self {
        self.inactive_opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Sets the same-color, lower-opacity band that leads the completed lyric fill.
    pub fn reveal_wave(mut self, reveal_wave: TimedTextRevealWave) -> Self {
        self.reveal_wave = Some(reveal_wave);
        self
    }

    /// Sets how far the lower-opacity leading fill spreads ahead of the
    /// completed fill.
    ///
    /// Smaller values keep the transition concentrated near the current
    /// playback position. This enables the reveal wave with its default
    /// settings if it was previously disabled.
    pub fn reveal_wave_width(mut self, width: Pixels) -> Self {
        self.reveal_wave.get_or_insert_default().width = width.max(px(0.));
        self
    }

    /// Sets the width of the feathered reveal edge.
    ///
    /// Smaller values produce a sharper edge without changing how far the
    /// lower-opacity fill spreads. This enables the reveal wave with its
    /// default settings if it was previously disabled.
    pub fn reveal_edge_softness(mut self, softness: Pixels) -> Self {
        self.reveal_wave.get_or_insert_default().softness = softness.max(px(0.));
        self
    }

    /// Restores the legacy hard reveal edge.
    pub fn without_reveal_wave(mut self) -> Self {
        self.reveal_wave = None;
        self
    }

    /// Sets the paint-only scale/lift animation for the active group.
    pub fn emphasis(mut self, emphasis: TimedTextEmphasis) -> Self {
        self.motion = TimedTextMotion::Elastic(emphasis);
        self
    }

    /// Lifts each word or phrase progressively over its playback interval.
    ///
    /// A group starts on the baseline, reaches the requested height exactly at
    /// its end time, and remains lifted afterward. The motion is paint-only and
    /// does not change text layout or push neighboring words.
    pub fn progressive_lift(mut self, height: Pixels) -> Self {
        self.motion = TimedTextMotion::ProgressiveLift(height.max(px(0.)));
        self
    }

    /// Selects the word or phrase groups that receive elastic emphasis.
    ///
    /// After elastic emphasis is enabled, every group participates by default.
    /// Calling this method limits it to the listed group identifiers. Other
    /// groups retain timed gradient reveal without a transform. Pass an empty
    /// iterator to disable elasticity for every group.
    pub fn elastic_groups(mut self, groups: impl IntoIterator<Item = usize>) -> Self {
        self.elastic_groups = Some(groups.into_iter().collect());
        self
    }

    /// Disables word/phrase motion while keeping timed reveal.
    pub fn without_emphasis(mut self) -> Self {
        self.motion = TimedTextMotion::None;
        self
    }
}

impl IntoElement for TimedText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for TimedText {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for TimedText {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Element for TimedText {
    type RequestLayoutState = TimedTextLayout;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(accesskit::Role::Label)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_value(self.text.to_string());
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = TimedTextLayout::default();
        let state_for_measure = state.clone();
        let text = self.text.clone();
        let inactive_opacity = self.inactive_opacity;

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, _cx| {
                window.with_text_style(style.text_style().cloned(), |window| {
                    let text_style = window.text_style();
                    let font_size = text_style.font_size.to_pixels(window.rem_size());
                    let line_height = window.pixel_snap(
                        text_style
                            .line_height
                            .to_pixels(font_size.into(), window.rem_size()),
                    );
                    let mut run = text_style.to_run(text.len());
                    run.color = run.color.opacity(inactive_opacity);

                    window.request_measured_layout(style, move |_, _, window, _cx| {
                        let line = window.text_system().shape_line(
                            text.clone(),
                            font_size,
                            std::slice::from_ref(&run),
                            None,
                        );
                        let measured_size = size(line.width().ceil(), line_height);
                        state_for_measure
                            .0
                            .borrow_mut()
                            .replace(TimedTextLayoutInner {
                                line,
                                line_height,
                                content_bounds: Bounds::new(Point::default(), measured_size),
                            });
                        measured_size
                    })
                })
            },
        );

        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let measured_size = state
            .0
            .borrow()
            .as_ref()
            .map(|layout| size(layout.line.width(), layout.line_height))
            .unwrap_or_default();

        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            measured_size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, _cx| {
                let padding = style
                    .padding
                    .to_pixels(bounds.size.into(), window.rem_size());
                if let Some(layout) = state.0.borrow_mut().as_mut() {
                    layout.content_bounds = Bounds::new(
                        bounds.origin + point(padding.left, padding.top) + scroll_offset,
                        size(
                            (bounds.size.width - padding.left - padding.right).max(px(0.)),
                            (bounds.size.height - padding.top - padding.bottom).max(px(0.)),
                        ),
                    );
                }
                hitbox
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |_style, window, cx| {
                let state = state.0.borrow();
                let Some(layout) = state.as_ref() else {
                    return;
                };

                let align = window.text_style().text_align;
                let align_width = layout.content_bounds.size.width;
                let transform = match self.motion {
                    TimedTextMotion::Elastic(emphasis) => active_transform(
                        &layout.line,
                        layout.line_height,
                        &self.units,
                        self.position,
                        emphasis,
                        self.elastic_groups.as_ref(),
                    ),
                    TimedTextMotion::ProgressiveLift(height) => {
                        progressive_lift_transforms(&self.units, self.position, height)
                    }
                    TimedTextMotion::None => Vec::new(),
                };
                let transforms = transform.as_slice();

                let aligned_x = match align {
                    TextAlign::Left => layout.content_bounds.origin.x,
                    TextAlign::Center => {
                        layout.content_bounds.origin.x
                            + (align_width - layout.line.width()).max(px(0.)) / 2.
                    }
                    TextAlign::Right => {
                        layout.content_bounds.origin.x
                            + (align_width - layout.line.width()).max(px(0.))
                    }
                };
                let fill_bounds = Bounds::new(
                    point(aligned_x, layout.content_bounds.origin.y),
                    size(layout.line.width().max(px(1.)), layout.line_height),
                );
                let inactive_color = window.text_style().color.opacity(self.inactive_opacity);

                // Both passes deliberately use grayscale atlas masks. Mixing
                // LCD subpixel glyphs with grayscale gradient glyphs produces
                // mismatched outlines at a partially revealed character.
                window.with_masked_fill(fill_bounds, inactive_color, |window| {
                    layout
                        .line
                        .paint_with_transforms(
                            layout.content_bounds.origin,
                            layout.line_height,
                            align,
                            Some(align_width),
                            transforms,
                            window,
                            cx,
                        )
                        .ok();
                });

                let reveal_x = reveal_x(&layout.line, &self.units, self.position);
                if reveal_x <= px(0.) {
                    return;
                }
                let vertical_overscan = match self.motion {
                    TimedTextMotion::ProgressiveLift(height) => height,
                    TimedTextMotion::Elastic(emphasis) => {
                        emphasis.translation.y.abs()
                            + layout.line_height * (emphasis.scale.max(1.0) - 1.0) / 2.
                    }
                    TimedTextMotion::None => px(0.),
                };
                let reveal_wave = self.reveal_wave.filter(|wave| wave.width > px(0.));
                let completed_x = reveal_wave
                    .map(|wave| (reveal_x - wave.width).max(px(0.)))
                    .unwrap_or(reveal_x);
                let completed_edge = completed_x.min(layout.line.width());
                let completed_clip = Bounds::new(
                    point(
                        aligned_x,
                        layout.content_bounds.origin.y - vertical_overscan,
                    ),
                    size(completed_edge, layout.line_height + vertical_overscan * 2.),
                );
                let solid_wave = reveal_wave.zip(self.active_fill.as_solid());

                if solid_wave.is_none() {
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: completed_clip,
                        }),
                        |window| {
                            window.with_masked_fill(fill_bounds, self.active_fill, |window| {
                                layout
                                    .line
                                    .paint_with_transforms(
                                        layout.content_bounds.origin,
                                        layout.line_height,
                                        align,
                                        Some(align_width),
                                        transforms,
                                        window,
                                        cx,
                                    )
                                    .ok();
                            });
                        },
                    );
                }

                if let Some((reveal_wave, color)) = solid_wave {
                    let line_width = layout.line.width().max(px(1.));
                    let color = Rgba::from(color);
                    let mut uniforms = EffectUniforms::default();
                    uniforms.set_slot(0, [color.r, color.g, color.b, color.a]);
                    uniforms.set_slot(
                        1,
                        [
                            (reveal_x / line_width).clamp(0.0, 1.0),
                            (completed_x / line_width).clamp(0.0, 1.0),
                            (reveal_wave.softness.max(px(0.5)) / line_width).clamp(0.0, 1.0),
                            reveal_wave.leading_opacity.clamp(0.0, 1.0),
                        ],
                    );
                    window.with_masked_effect(
                        fill_bounds,
                        timed_text_opacity_reveal_shader(),
                        uniforms,
                        0.0,
                        1.0,
                        |window| {
                            layout
                                .line
                                .paint_with_transforms(
                                    layout.content_bounds.origin,
                                    layout.line_height,
                                    align,
                                    Some(align_width),
                                    transforms,
                                    window,
                                    cx,
                                )
                                .ok();
                        },
                    );
                } else if let Some(reveal_wave) = reveal_wave {
                    let wave_width = (reveal_x - completed_x).max(px(0.));
                    let steps = ((wave_width / px(2.)).ceil() as usize).clamp(4, 12);
                    let softness = reveal_wave.softness.max(px(0.5));
                    for step in 0..steps {
                        let start_t = step as f32 / steps as f32;
                        let end_t = (step + 1) as f32 / steps as f32;
                        let center_t = (start_t + end_t) * 0.5;
                        let strip_start = completed_x + wave_width * start_t;
                        let strip_end = completed_x + wave_width * end_t;
                        let opacity = (1.0
                            - (1.0 - reveal_wave.leading_opacity.clamp(0.0, 1.0))
                                * smootherstep(center_t))
                            * smootherstep((reveal_x - (strip_start + strip_end) / 2.) / softness);
                        if opacity <= f32::EPSILON {
                            continue;
                        }
                        let strip_clip = Bounds::new(
                            point(
                                aligned_x + strip_start,
                                layout.content_bounds.origin.y - vertical_overscan,
                            ),
                            size(
                                (strip_end - strip_start + px(0.75)).min(reveal_x - strip_start),
                                layout.line_height + vertical_overscan * 2.,
                            ),
                        );
                        window.with_content_mask(
                            Some(ContentMask { bounds: strip_clip }),
                            |window| {
                                window.with_masked_fill(
                                    fill_bounds,
                                    self.active_fill.opacity(opacity),
                                    |window| {
                                        layout
                                            .line
                                            .paint_with_transforms(
                                                layout.content_bounds.origin,
                                                layout.line_height,
                                                align,
                                                Some(align_width),
                                                transforms,
                                                window,
                                                cx,
                                            )
                                            .ok();
                                    },
                                );
                            },
                        );
                    }
                }
            },
        );
    }
}

fn timed_text_opacity_reveal_shader() -> EffectShader {
    EffectShader::wgsl_mask(include_str!("shaders/timed_text_opacity_reveal.wgsl"))
}

fn validate_units(text: &str, units: &[TimedTextUnit]) {
    let mut previous_start = Duration::ZERO;
    for (index, unit) in units.iter().enumerate() {
        assert!(
            unit.start <= unit.end,
            "timed text unit starts after it ends"
        );
        assert!(
            unit.range.start <= unit.range.end,
            "timed text range is reversed"
        );
        assert!(
            unit.range.end <= text.len(),
            "timed text range is outside the line"
        );
        assert!(
            text.is_char_boundary(unit.range.start) && text.is_char_boundary(unit.range.end),
            "timed text range must use UTF-8 byte boundaries"
        );
        if index > 0 {
            assert!(
                previous_start <= unit.start,
                "timed text units must be chronological"
            );
        }
        previous_start = unit.start;
    }
}

fn duration_progress(position: Duration, start: Duration, end: Duration) -> f32 {
    if end <= start {
        return f32::from(position >= end);
    }
    ((position.saturating_sub(start).as_secs_f64() / (end - start).as_secs_f64()) as f32)
        .clamp(0.0, 1.0)
}

fn smootherstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}

fn group_is_elastic(groups: Option<&BTreeSet<usize>>, group: usize) -> bool {
    groups.is_none_or(|groups| groups.contains(&group))
}

fn reveal_x(line: &ShapedLine, units: &[TimedTextUnit], position: Duration) -> Pixels {
    let mut reached = px(0.);
    for unit in units {
        if position < unit.start {
            break;
        }
        let start_x = line.x_for_index(unit.range.start);
        let end_x = line.x_for_index(unit.range.end);
        if position < unit.end {
            return start_x + (end_x - start_x) * duration_progress(position, unit.start, unit.end);
        }
        reached = end_x;
    }
    reached
}

fn progressive_lift_transforms(
    units: &[TimedTextUnit],
    position: Duration,
    height: Pixels,
) -> Vec<GlyphRunTransform> {
    struct GroupTiming {
        id: usize,
        range: Range<usize>,
        start: Duration,
        end: Duration,
    }

    let mut groups: Vec<GroupTiming> = Vec::new();
    for unit in units {
        if let Some(group) = groups.iter_mut().find(|group| group.id == unit.group) {
            group.range.start = group.range.start.min(unit.range.start);
            group.range.end = group.range.end.max(unit.range.end);
            group.start = group.start.min(unit.start);
            group.end = group.end.max(unit.end);
        } else {
            groups.push(GroupTiming {
                id: unit.group,
                range: unit.range.clone(),
                start: unit.start,
                end: unit.end,
            });
        }
    }

    groups
        .into_iter()
        .filter_map(|group| {
            if position < group.start || height <= px(0.) {
                return None;
            }
            let amount = smootherstep(duration_progress(position, group.start, group.end));
            if amount <= f32::EPSILON {
                return None;
            }
            Some(GlyphRunTransform::new(
                group.range,
                Point::default(),
                Transformation::translate(point(px(0.), -height * amount)),
            ))
        })
        .collect()
}

fn active_transform(
    line: &ShapedLine,
    line_height: Pixels,
    units: &[TimedTextUnit],
    position: Duration,
    emphasis: TimedTextEmphasis,
    elastic_groups: Option<&BTreeSet<usize>>,
) -> Vec<GlyphRunTransform> {
    let Some(active) = units
        .iter()
        .find(|unit| position >= unit.start && position < unit.end)
    else {
        return Vec::new();
    };
    if !group_is_elastic(elastic_groups, active.group) {
        return Vec::new();
    }

    let mut group_range = active.range.clone();
    let mut group_start = active.start;
    let mut group_end = active.end;
    for unit in units.iter().filter(|unit| unit.group == active.group) {
        group_range.start = group_range.start.min(unit.range.start);
        group_range.end = group_range.end.max(unit.range.end);
        group_start = group_start.min(unit.start);
        group_end = group_end.max(unit.end);
    }

    let progress = duration_progress(position, group_start, group_end);
    let enter = smootherstep(progress / emphasis.enter_fraction.max(f32::EPSILON));
    let exit = smootherstep((1.0 - progress) / emphasis.exit_fraction.max(f32::EPSILON));
    let amount = enter.min(exit);
    let scale_value = 1.0 + (emphasis.scale.max(0.0) - 1.0) * amount;
    let translation = emphasis.translation * amount;
    let start_x = line.x_for_index(group_range.start);
    let end_x = line.x_for_index(group_range.end);
    let group_start_index = group_range.start;
    let group_end_index = group_range.end;
    let scaled_half_growth = (end_x - start_x) * (scale_value - 1.0) / 2.;
    let side_push = scaled_half_growth + emphasis.surrounding_spread.max(px(0.)) * amount;
    if scale_value == 1.0 && translation == Point::default() && side_push == px(0.) {
        return Vec::new();
    }

    let baseline = (line_height - line.ascent - line.descent) / 2. + line.ascent;
    let mut transforms = vec![GlyphRunTransform::new(
        group_range,
        point((start_x + end_x) / 2., baseline),
        Transformation::scale(size(scale_value, scale_value)).with_translation(translation),
    )];
    if group_start_index > 0 {
        transforms.push(GlyphRunTransform::new(
            0..group_start_index,
            Point::default(),
            Transformation::translate(point(-side_push, px(0.))),
        ));
    }
    if group_end_index < line.len() {
        transforms.push(GlyphRunTransform::new(
            group_end_index..line.len(),
            Point::default(),
            Transformation::translate(point(side_push, px(0.))),
        ));
    }
    transforms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_progress_handles_variable_and_zero_length_units() {
        assert_eq!(
            duration_progress(
                Duration::from_millis(350),
                Duration::from_millis(100),
                Duration::from_millis(600)
            ),
            0.5
        );
        assert_eq!(
            duration_progress(
                Duration::from_millis(100),
                Duration::from_millis(100),
                Duration::from_millis(100)
            ),
            1.0
        );
    }

    #[test]
    fn validates_utf8_byte_boundaries() {
        let result = std::panic::catch_unwind(|| {
            TimedText::new(
                "你a",
                [TimedTextUnit::new(
                    1..3,
                    Duration::ZERO,
                    Duration::from_secs(1),
                )],
            );
        });
        assert!(result.is_err());
    }

    #[test]
    fn elastic_group_allow_list_is_caller_controlled() {
        assert!(group_is_elastic(None, 9));
        let groups = BTreeSet::from([1, 3]);
        assert!(group_is_elastic(Some(&groups), 3));
        assert!(!group_is_elastic(Some(&groups), 2));
        assert!(!group_is_elastic(Some(&BTreeSet::new()), 1));
    }

    #[test]
    fn progressive_lift_rises_over_the_group_and_then_holds() {
        let units = [
            TimedTextUnit::new(0..2, Duration::ZERO, Duration::from_millis(500)).group(0),
            TimedTextUnit::new(
                2..4,
                Duration::from_millis(500),
                Duration::from_millis(1000),
            )
            .group(0),
            TimedTextUnit::new(
                5..9,
                Duration::from_millis(1000),
                Duration::from_millis(2000),
            )
            .group(1),
        ];
        let height = px(4.);

        assert!(progressive_lift_transforms(&units, Duration::ZERO, height).is_empty());
        assert_eq!(
            progressive_lift_transforms(&units, Duration::from_millis(500), height),
            vec![GlyphRunTransform::new(
                0..4,
                Point::default(),
                Transformation::translate(point(px(0.), px(-2.))),
            )]
        );
        assert_eq!(
            progressive_lift_transforms(&units, Duration::from_millis(1500), height),
            vec![
                GlyphRunTransform::new(
                    0..4,
                    Point::default(),
                    Transformation::translate(point(px(0.), px(-4.))),
                ),
                GlyphRunTransform::new(
                    5..9,
                    Point::default(),
                    Transformation::translate(point(px(0.), px(-2.))),
                ),
            ]
        );
        assert_eq!(
            progressive_lift_transforms(&units, Duration::from_millis(2500), height),
            vec![
                GlyphRunTransform::new(
                    0..4,
                    Point::default(),
                    Transformation::translate(point(px(0.), px(-4.))),
                ),
                GlyphRunTransform::new(
                    5..9,
                    Point::default(),
                    Transformation::translate(point(px(0.), px(-4.))),
                ),
            ]
        );
    }
}
