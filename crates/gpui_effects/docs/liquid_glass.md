# Liquid glass

`LiquidGlass` is a screen-space refractive material. It samples content already
painted in the same GPUI window, bends it near curved edges, and adds directional
reflection, an optional background-colored border, and inner shading. The center blends sharp and blurred background
samples. Foreground children remain sharp.

The material provides no component behavior, popup management, or automatic
foreground contrast selection. It supports rounded rectangles with independent
corner radii. Shape merging and arbitrary path silhouettes are not supported.

## Material configuration

```rust
use gpui::{point, px};
use gpui_effects::LiquidGlassAppearance;

let material = LiquidGlassAppearance::regular()
    .blur_radius(px(8.0))
    .clarity(0.22)
    .refraction(px(7.0))
    .thickness(px(16.0))
    .highlight(0.34)
    .dispersion(0.015)
    .light_direction(point(-0.6, -0.8));
```

`regular()` is the default softened surface. `clear()` retains more background
detail with less blur and tint. `dark()` adds a dark neutral wash. These are
starting configurations, not automatic adaptations to the background.

| Field | Regular default | Meaning |
| --- | --- | --- |
| `blur_radius` | `8 px` | Nonnegative background blur radius |
| `clarity` | `0.22` | Sharp background contribution, `0..=1`; `1` is fully sharp |
| `refraction` | `7 px` | Inward sampling displacement at the rim; `0` disables bending |
| `thickness` | `16 px` | Width of the curved edge region; `0` disables curvature |
| `dispersion` | `0.015` | Relative red/blue displacement, `0..=0.1`; `0` disables separation |
| `tint` | White, alpha `0.12` | Color wash mixed into the sampled background |
| `saturation` | `1.04` | Nonnegative background saturation multiplier |
| `brightness` | `1.0` | Nonnegative background brightness multiplier |
| `highlight` | `0.34` | Directional reflection strength, `0..=1` |
| `edge_shadow` | `0.04` | Inner edge shading, `0..=1`; not a cast shadow |
| `rim_width` | `0.8 px` | Fine reflective rim width; `0` disables the fine rim |
| `edge_tint_strength` | `0.0` | Strength of the background-tinted border reflection, `0..=1`; opt-in, `0` disables it |
| `edge_tint_width` | `1 px` | Width of the inset background-colored outline; `0` disables it |
| `edge_sample_distance` | `8 px` | Nonnegative sampling distance beyond the silhouette |
| `edge_tint_lift` | `0.35` | Brightness lift preserving sampled RGB ratios, `0..=1`; `0` retains sampled brightness, `1` brings the peak channel to full brightness on the lit side |
| `light_direction` | `(-0.6, -0.8)` | Direction toward the light; zero uses the default direction |

Lengths use logical pixels and follow the window scale factor. Edge thickness
is limited to 45% of the smaller surface dimension. Refraction is limited to
45% of that effective thickness to limit edge distortion. Keep dispersion low for
a neutral material; it affects only the refracted edge, not the entire surface.

`clarity` controls optical sharpness, not element opacity. Use `tint` for a
material color wash and normal GPUI opacity for the entire element, including
its foreground. Foreground color and readability remain the caller's choice.

## Background-colored border

The background-colored border is disabled in all presets. Set `edge_tint_strength`
above zero to enable them; the default width and sample distance can be used
without additional configuration.

The border gathers raw background colors just outside the silhouette and on
either side along its tangent. `edge_tint_lift` raises brightness by scaling all
three sampled channels together, preserving their ratios. It does not mix in
white or increase saturation. A black sample remains black. `edge_tint_strength`
controls the border's contribution, while `light_direction` and `highlight`
vary the brightness lift around the outline, not its opacity.

The ordinary white rim is attenuated beneath the colored stroke. At strength
`1`, the interior of the stroke uses the lifted sampled RGB directly; lower
strengths blend it with the surface. The contour's antialiasing still blends
its boundary pixels with the surrounding scene.

The reflection uses a constant-width inset stroke with an antialiased inner
edge, not a broad gradient across the surface. Its width is limited to half the
smaller surface dimension. These border parameters do not change refraction.

```rust
use gpui::px;
use gpui_effects::LiquidGlassAppearance;

let material = LiquidGlassAppearance::regular()
    .edge_tint_strength(0.6)
    .edge_tint_width(px(1.0))
    .edge_sample_distance(px(12.0))
    .edge_tint_lift(0.35)
    .dispersion(0.0);
```

This is separate from RGB dispersion and does not require refraction. It follows
the background rather than a fixed border color. Neutral backgrounds remain
neutral, and a uniform background naturally produces less color variation.
Gathering is independent of `blur_radius`. Set `edge_tint_strength(0.0)`
or `edge_tint_width(px(0.0))` to disable gathering without changing other optics.
Layout bounds and the center of the surface are unchanged. This is a material
outline; it does not allocate a conventional GPUI layout border.

## Paint into an existing element

`paint_liquid_glass` is independent of layout and interaction. Call it before
painting foreground content:

```rust
use gpui::{div, prelude::*, px};
use gpui_effects::{LiquidGlassAppearance, paint_liquid_glass};

let material = LiquidGlassAppearance::regular();
let surface = div()
    .w(px(320.0))
    .h(px(200.0))
    .rounded(px(28.0))
    .on_paint_before_children(move |bounds, style, window, _| {
        let corners = style.corner_radii.to_pixels(window.rem_size());
        paint_liquid_glass(bounds, corners, material, window);
    })
    .child("Foreground content");
```

The painter clamps corner radii to the supplied bounds. It owns no element
state, input handling, clipping of children, or foreground styling.

## Styled surface wrapper

`LiquidGlass` supplies the same material with a Div-backed surface:

```rust
use gpui::{prelude::*, px, rgb};
use gpui_effects::{LiquidGlass, LiquidGlassAppearance};

let surface = LiquidGlass::with_appearance(LiquidGlassAppearance::clear())
    .w(px(360.0))
    .h(px(220.0))
    .rounded(px(32.0))
    .p_6()
    .text_color(rgb(0x1c3049))
    .child("Sharp foreground content");
```

Layout, typography, individual corner radii, shadows, borders, and child
clipping use `Styled`. Assign `.id(...)` before stateful interaction such as
`.on_click(...)` or `.hover(...)`. The wrapper does not add padding, radius,
foreground colors, or a drop shadow by default. A normal Div background is
painted before the material and participates in its backdrop; prefer the
material's `tint` when the goal is to color the glass itself.

## Material study

```sh
cargo run -p gpui_effects --example liquid_glass --features gpui_platform/runtime_shaders
```

The study contains two independently draggable glass surfaces over a gradient
and optional grid: a 460 × 280 surface and a smaller 128 × 80 surface. They share
the material configuration and requested corner radius; each surface clamps
its corners to its own bounds. Controls change presets, blur, clarity,
refraction, thickness, highlight,
dispersion, edge tint strength/width/sampling distance, corner radius, and
background. Border color starts OFF. The Border color button enables it on both
surfaces at strength `1.0`; turning it off and on restores the last enabled
strength. The Border strength, Border width, Border sampling, and Border brightness controls tune
the outline. Preset selection restores its disabled default. The control area
scrolls when needed. Moving the surface reveals how
background lines bend while foreground text remains unchanged. It requests
redraws on interaction rather than running a continuous animation loop.

Backdrop sampling is limited to previously painted content in the same window;
it does not capture the desktop or another native window. Rendering requires
backend support for custom backdrop shaders. Each surface adds backdrop capture
and filtering work; share a surface where appropriate rather than painting
many redundant overlapping materials. `FrostedGlass` remains a separate material.
