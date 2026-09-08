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

Pixel dimensions follow the window scale factor. Blur and wave reserve capture
padding automatically; `.capture_padding(...)` replaces that padding. Blur uses
a fixed 7 × 7 Gaussian kernel intended for small UI radii; large radii spread
the samples farther apart. Color adjustment preserves alpha and operates on the
renderer’s sampled RGB values.

Wave speed is measured in radians per second; negative speed reverses motion.
Wave does not start its own animation loop. Update time and request frames while animating.

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

- `uniforms` replaces the shader's uniform slots; `uniform` updates one slot.
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

Render targets stay on the GPU and are reused at each nesting depth. Each
target currently uses the window's device-pixel dimensions and surface format.
Window resizing recreates the targets. Captured content is repainted when the
window renders; it is not a persistent snapshot cache.

## Example

```sh
cargo run -p gpui_effects --example subtree_effect
```

The example compares original and captured content in identity, blur, wave
and color modes. The controls adjust effect strength.
The card buttons remain interactive. Pause stops the wave animation.
