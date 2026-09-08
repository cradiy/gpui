# Subtree effects

`SubtreeEffect` captures an element and its painted children into a transparent
GPU texture, then composites the result through an image shader. Text, images,
backgrounds, borders and nested effects share the same input surface.

## Built-in effects

```rust,ignore
use gpui::{div, prelude::*, px};
use gpui_effects::{
    SubtreeColorOptions, SubtreeWaveOptions,
    subtree_blur, subtree_color_adjust, subtree_wave,
};

let blurred = subtree_blur(div().child("Soft focus"), px(4.));

let waving = subtree_wave(
    div().child("Moving content"),
    SubtreeWaveOptions {
        amplitude: px(6.),
        wavelength: px(180.),
        speed: 1.5,
    },
).time(2.0);

let grayscale = subtree_color_adjust(
    div().child("Quiet colors"),
    SubtreeColorOptions { saturation: 0., ..Default::default() },
);
```

| Effect | Parameters | Neutral state |
| --- | --- | --- |
| `subtree_identity` | None | Captured content unchanged |
| `subtree_blur` | Support radius in logical pixels | Radius `0` |
| `subtree_wave` | Amplitude, wavelength, speed; `.time(seconds)` | Amplitude `0` |
| `subtree_color_adjust` | Saturation, contrast, brightness | All `1` |
| `subtree_bloom` | Threshold, soft knee, intensity, radius, downsample | Intensity or radius `0` |

Pixel dimensions follow the window scale factor. Blur and wave reserve capture
padding automatically; `.capture_padding(...)` replaces that padding. Blur uses
a fixed 7 × 7 Gaussian kernel intended for small UI radii; large radii spread
the samples farther apart. Color adjustment preserves alpha and operates on the
renderer’s sampled RGB values.

Wave speed is measured in radians per second; negative speed reverses motion.
Wave does not start its own animation loop. Update time and request frames while animating.

## Bloom

```rust,ignore
use gpui::{div, prelude::*, px, rgb};
use gpui_effects::{BloomOptions, subtree_bloom};

let title = subtree_bloom(
    div().text_size(px(48.)).text_color(rgb(0x9aeeff)).child("Luminous"),
    BloomOptions {
        threshold: 0.6,
        soft_knee: 0.15,
        intensity: 1.6,
        radius: px(56.),
        downsample: 4,
    },
);
```

Bloom extracts highlights, applies horizontal and vertical Gaussian passes, then
screen-blends the colored glow with the unblurred input. It affects bright text, images and
other painted content without replacing their sharp details.
The glow contribution tapers on bright source pixels and remains full in dark
or transparent regions.

- `threshold`: highlight cutoff from `0` to `1`, evaluated on sampled RGB brightness.
- `soft_knee`: transition width around the cutoff; `0` gives a hard threshold.
  Highlights above this transition contribute their full color to the glow.
- `intensity`: glow contribution; `0` omits the stage.
- `radius`: Gaussian support radius in logical pixels; `0` omits the stage.
- `downsample`: texture size divisor, clamped to `1..=8`. The default `4` uses
  quarter-width, quarter-height textures. Use `1` or `2` for finer highlights.

Capture padding includes the glow radius and sampling margin. Parent clipping
still applies, so allow room around the content. The composite uses the renderer's
surface format; colors above its range are clamped, not HDR tone-mapped.

Use `EffectStage::bloom(options)` in a chain. `bloom_extract_shader`,
`bloom_blur_shader` and `bloom_composite_shader` expose the individual WGSL stages.

## Effect chains

```rust,ignore
use gpui::{div, prelude::*, px};
use gpui_effects::{EffectStage, SubtreeColorOptions, subtree_effect_chain};

let content = subtree_effect_chain(
    div().p_4().child("Layered content"),
    [
        EffectStage::blur(px(4.)),
        EffectStage::color_adjust(SubtreeColorOptions {
            saturation: 0.7,
            contrast: 1.4,
            ..Default::default()
        }),
    ],
).effect_opacity(0.9);
```

Stages run in iteration order. `EffectStage::identity`, `blur`, `wave`,
`color_adjust` and `bloom` provide built-in stages; `EffectStage::new(shader)` accepts a
custom single-image shader. Each stage has its own uniforms, logical-pixel
uniforms and padding. `.enabled(false)` omits that stage entirely.

`Feedback::stage()` supplies a persistent history stage. See
[History feedback](feedback.md) for input capture, decay and playback controls.

`EffectStage::ripples(options, waves)` applies local radial displacement. See
[Water ripple](ripple.md) for wave coordinates and playback.

`EffectStage::lens(options)` magnifies or compresses a local region. See
[Local lens](lens.md) for pointer tracking and falloff controls.

`EffectStage::displacement_map(map, options)` samples an external RG control map.
`EffectStage::with_images(shader, images)` binds external images after the captured
source for custom two-image or four-image shaders. See
[Displacement maps](displacement_map.md) for map formats, masking and sampling.

An existing effect can append a stage with `.then(EffectStage::blur(px(2.)))`.
The wrapper's `uniform`, `uniforms` and `uniform_pixels` methods configure its
first stage. Configure subsequent stages before appending them. `.time(seconds)`
supplies a common clock to all stages; each animated shader can define its own speed.

The content is painted once per capture. Stage outputs alternate between
two full-resolution GPU textures, independent of stage count. Bloom also uses
two reduced-resolution textures for highlight extraction and blur. The final
composite applies chain opacity when drawing to the parent target. An empty
or fully disabled chain paints directly, without allocating capture targets.

Active stages' padding is accumulated. A wrapper-level `capture_padding` replaces
the accumulated value at that point; further `.then(...)` calls add their padding.
Stages share the capture bounds; Bloom uses reduced-resolution intermediate textures. Changing
stage order can change the output, especially with clipping or nonlinear color operations.

## Custom shaders

The `subtree_*_shader()` functions expose each built-in WGSL shader separately.
Their API documentation specifies the uniform slots. Custom shaders use the
same single-image contract:

```rust,ignore
use gpui::{EffectShader, div, prelude::*, px};
use gpui_effects::subtree_effect;

let shader = EffectShader::wgsl_image(r#"
fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    return sample_effect_image(input, input.uv);
}
"#);

let content = subtree_effect(
    div().p_4().child("Captured content"),
    shader,
)
.capture_padding(px(16.))
.effect_opacity(0.8);
```

## Configuration

- `uniforms` replaces the first stage's uniform slots; `uniform` updates one slot.
- `uniform_pixels` sets a slot in logical pixels, converted at paint time.
  `uniform` removes pixel conversion for that slot; `uniforms` removes it for all slots.
- `time` supplies elapsed seconds. The component does not schedule animation frames.
- `capture_padding` adds paint-only space around the element for shadows,
  blur and displaced pixels. It does not change layout.
- `effect_opacity` controls the final composite.
- `enabled(false)` paints the wrapped element directly.
- `Styled`, child composition and supported interaction methods delegate to
  the wrapped element.

## Coordinates and transparency

`input.uv` spans the capture bounds, including padding. `input.size` is the
capture size in device pixels. `sample_effect_image` returns straight-alpha
colors and treats pixels beyond the capture bounds as transparent. Interpolation
uses premultiplied colors internally to preserve transparent edges.

When averaging samples for blur, accumulate `sample.rgb * sample.a` and alpha
separately, then divide the accumulated RGB by the accumulated alpha.

Parent clipping still applies. Layout, accessibility and pointer hit testing
retain their original coordinates. Shader displacement changes visual pixels;
it does not relocate interactive targets. Deferred overlays are painted outside
the capture.

Nested captures are supported. Backdrop effects inside a capture sample earlier
content within that capture. Text uses grayscale antialiasing on the transparent
surface.

## Renderer support

Linux Wayland and X11 windows support subtree effects through WGPU. Use
`window.supports_subtree_effects()` to query availability. Unsupported window
backends paint the original content with the configured effect opacity.

Render targets stay on the GPU and are reused across captures and effect passes. Each
capture target uses the window's device-pixel dimensions and surface format.
Bloom reuses a pair of reduced-resolution `RGBA16Float` textures per active
downsample divisor to preserve faint highlights during filtering.
Window resizing recreates the targets. Captured content is repainted when the
window renders; it is not a persistent snapshot cache.

## Example

```sh
cargo run -p gpui_effects --example subtree_effect
```

The example compares original and captured content in identity, blur, wave,
color and chain modes. Chain mode combines blur and color adjustment, with
individual stage toggles and an order switch. The controls adjust effect strength.
The card buttons remain interactive. Pause stops the wave animation.

```sh
cargo run -p gpui_effects --example bloom
```

The Bloom example compares text and artwork with the original content. Controls
adjust highlight extraction, intensity, radius and intermediate resolution.
