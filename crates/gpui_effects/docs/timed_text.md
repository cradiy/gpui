# Timed text

`TimedText` is a single-line text element for karaoke lyrics, timed captions,
and playback-synchronized labels. It supports progressive solid or gradient
fills, cumulative lyric lift, and elastic emphasis for selected words or
phrases.

The complete line is shaped once. Timed units and elastic groups are painted
from the resulting glyph data, without creating a GPUI element for every word.

## Quick start

```rust,ignore
use std::time::Duration;
use gpui::{linear_gradient, px, rgb};
use gpui_effects::{TimedText, TimedTextUnit};

let text = "hello world";
let units = [
    TimedTextUnit::new(
        0..5,
        Duration::from_millis(120),
        Duration::from_millis(780),
    )
    .group(0),
    TimedTextUnit::new(
        6..11,
        Duration::from_millis(1050),
        Duration::from_millis(1840),
    )
    .group(1),
];

TimedText::new(text, units)
    .position(playback_position)
    .active_fill(linear_gradient(90., rgb(0x67e8f9), rgb(0xf9a8d4)))
    .inactive_opacity(0.28)
    .progressive_lift(px(3.))
    .text_size(px(32.))
```

Update `.position(...)` with the media playback position whenever the view is
rendered. Each unit may use a different duration, and gaps between units are
allowed.

## Timed units

A `TimedTextUnit` associates a substring with a playback interval:

```rust,ignore
TimedTextUnit::new(byte_range, start, end).group(group_id)
```

- `byte_range` is a UTF-8 byte range into the complete line.
- `start` and `end` use the same time base as `.position(...)`.
- `group_id` identifies the word or phrase used for elastic emphasis.

Ranges must start and end on UTF-8 boundaries. A unit may represent one
grapheme, one word, or any other timed substring. Units are ordered by their
start time when `TimedText` is created.

Whitespace has no segmentation role. Visible language boundaries come from
the supplied ranges and group identifiers, so continuous scripts such as
Chinese, Japanese, and Thai do not require inserted spaces. Ordinary spaces in
English text retain their layout width and do not require separate timing
units.

### Character timing with word emphasis

Assign multiple character units to the same group when timing is available per
character but emphasis should apply to the complete word:

```rust,ignore
let units = [
    TimedTextUnit::new(first_range, first_start, first_end).group(4),
    TimedTextUnit::new(second_range, second_start, second_end).group(4),
    TimedTextUnit::new(third_range, third_start, third_end).group(4),
];
```

The characters reveal according to their individual intervals. Scale, lift,
and surrounding displacement use the union of the ranges in group `4`.

## Progressive lyric lift

`.progressive_lift(px(...))` raises every group smoothly over its complete
playback interval. A group begins on the baseline, reaches the requested
height at its end time, and remains there. Previously completed groups stay
lifted while the current group continues rising.

The movement is derived entirely from `.position(...)`, so pausing freezes the
motion and seeking immediately reconstructs the correct state. It is a
paint-only Y translation and does not reshape the line or move neighboring
words.

## Selecting elastic groups

Motion is disabled by default. After enabling elastic emphasis with
`.emphasis(...)`, all groups participate unless `.elastic_groups(...)` supplies
an allow-list:

```rust,ignore
TimedText::new(text, units)
    .emphasis(TimedTextEmphasis::default())
    .elastic_groups([0, 2, 5])
```

Groups `0`, `2`, and `5` scale, lift, and push neighboring text. Other groups
continue to use timed fill without changing shape or position. An empty list
disables elasticity for the complete line:

```rust,ignore
TimedText::new(text, units)
    .emphasis(TimedTextEmphasis::default())
    .elastic_groups([])
```

The list can be generated from lyric metadata. For example, to emphasize
groups whose duration is at least 350 milliseconds:

```rust,ignore
let elastic_groups = words
    .iter()
    .filter(|word| word.end - word.start >= Duration::from_millis(350))
    .map(|word| word.group);

TimedText::new(text, units)
    .emphasis(TimedTextEmphasis::default())
    .elastic_groups(elastic_groups)
```

## Elastic emphasis

`TimedTextEmphasis` controls the active group's visual motion:

| Field | Unit | Description |
| --- | --- | --- |
| `scale` | ratio | Peak group scale. `1.0` disables scaling. |
| `translation` | logical pixels | Peak offset. A negative Y value lifts the group. |
| `surrounding_spread` | logical pixels per side | Additional separation applied to the text on both sides. Width added by scaling is compensated automatically. |
| `enter_fraction` | `0.0..1.0` | Fraction of group duration used to ease into the effect. |
| `exit_fraction` | `0.0..1.0` | Fraction of group duration used to ease out of the effect. |

The active group scales around one shared baseline-centered anchor. Its prefix
moves left and its suffix moves right, then all three ranges ease back into
place. These are paint-only transforms: the line is not reshaped or laid out
again on every frame.

Use `.without_emphasis()` to retain timed fill while disabling scale, lift,
and surrounding displacement.

## Fill and text styling

| Method | Description |
| --- | --- |
| `.active_fill(...)` | Solid color or gradient used for reached text. |
| `.inactive_opacity(0.0..1.0)` | Opacity of unreached text relative to the inherited text color. |
| `.position(Duration)` | Playback position represented by the current frame. |
| `.elastic_groups(...)` | Groups that receive elastic emphasis. |
| `.reveal_wave(...)` | Width, leading opacity, and edge softness of the progressive fill. |
| `.reveal_wave_width(px(...))` | Controls how far the lower-opacity fill spreads ahead of completed text. |
| `.reveal_edge_softness(px(...))` | Controls edge feathering without changing the leading range. |
| `.without_reveal_wave()` | Uses a hard reveal boundary. |


`TimedText` implements `Styled` and `InteractiveElement`. Standard GPUI text
and layout methods such as `.text_size(...)`, `.font_weight(...)`, `.id(...)`,
padding, and fixed sizing can be applied directly.

## Playback integration

For media playback, use the player's clock as `.position(...)` and request a
new frame as the clock advances. This keeps lyric progress synchronized with
seeking, pausing, and playback-rate changes.

For a self-running preview, `with_animation` can map a repeating animation
phase into the line duration:

```rust,ignore
TimedText::new(text, units).with_animation(
    "timed-text-preview",
    Animation::new(line_duration).repeat(),
    move |line, phase| line.position(line_duration.mul_f32(phase)),
)
```

## Example

The example contains synchronized Chinese, English, and mixed-script lines.
It also demonstrates elastic and non-elastic groups on the same timeline:

```sh
cargo run -p gpui_effects --example timed_text
```
