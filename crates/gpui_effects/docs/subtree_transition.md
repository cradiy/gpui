# Subtree transitions

`subtree_transition` composites two element subtrees through an independent
two-input GPU capture. Text, images and decorations transition together.

```rust
use gpui::{div, prelude::*, px};
use gpui_effects::{TransitionKind, subtree_transition};

let from = div().size_full().child("Evening collection");
let to = div().size_full().child("Morning collection");
let content = subtree_transition("collection", from, to)
    .w(px(640.))
    .h(px(360.))
    .kind(TransitionKind::BlurFade)
    .blur_radius(px(8.))
    .progress(0.4);
```

Give the transition a stable ID and an explicit size, or fill a bounded parent.
Both inputs occupy the same content area. Standard `Styled` methods configure the
outer container; backgrounds and borders on that container do not transition.
Style each input separately to transition its background or decorations.

## Presets

- `BlurFade` is the default. The outgoing input blurs as it fades, while the
  incoming input becomes clear. `blur_radius` controls the maximum support radius
  in logical pixels, from 0 to 24; the default is 8.
- `CrossFade` blends colors and alpha without spatial filtering.
- `WipeRight`, `WipeLeft`, `WipeDown` and `WipeUp` reveal the incoming input in the
  named direction. `edge_softness` controls the relative half-width of the soft
  edge, from 0.001 to 0.5; the default is 0.16.

The compositor interpolates premultiplied colors and applies ancestor opacity
once to the result. Transparent input regions remain transparent.

## Playback

Progress is caller-controlled and clamped to 0..=1. Zero shows only `from`; one
shows only `to`. Decreasing progress reverses the transition. Apply easing before
passing progress when desired.

The component does not own a timer or request animation frames. Update progress
and notify the containing view during playback. At rest, keep progress unchanged.
At either endpoint only the visible input is laid out and painted, without the
two-input capture. Input content is live during intermediate frames, not a frozen
snapshot. Replacing either input during playback replaces its rendered content.

## Input and rendering boundaries

During intermediate frames, the content area blocks ordinary pointer hit testing.
At an endpoint only the visible input participates. Keep playback controls outside
the transition. Keyboard focus, accessibility and custom global input listeners
remain application-managed; move focus out of outgoing content when appropriate.

Inputs are clipped to their shared rectangular content bounds. Leave space inside
that area for blur or displaced edges. Deferred overlays paint outside the capture
and do not participate in the transition. Nested subtree effects and transitions
are supported; stateful GPU resources in the two inputs need distinct identities.

WGPU backends with subtree-effect support use the GPU compositor. Unsupported
backends show `from` below progress 0.5 and `to` at or above 0.5.

## Low-level composition

`Window::with_subtree_pair` captures its callback once for `SubtreeInput::First`
and once for `SubtreeInput::Second`. Both textures share snapped capture bounds.
Prepaint each input inside `Window::prepaint_subtree_effect`.

Use an `EffectShader::wgsl_two_images` shader with `sample_effect_image` and
`sample_effect_second_image`. These helpers return straight-alpha samples from
premultiplied capture textures. `transition_shader` exposes the built-in shaders;
its uniform layout is documented on the function.

## Example

```sh
cargo run -p gpui_effects --example subtree_transition
```

Switch presets, use Previous or Next to animate, pause playback, or scrub the
progress bar to inspect intermediate frames.
