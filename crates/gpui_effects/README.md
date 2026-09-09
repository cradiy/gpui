# gpui_effects

GPU-driven visual effects and reusable effect components for GPUI applications.
WGSL is the canonical shader implementation, with GPUI providing the render
pipeline and `gpui_effects` providing higher-level components and presets.

## Guides

- [Frosted glass](docs/glass.md): strongly blurred panels and mergeable rounded surfaces.
- [Timed text](docs/timed_text.md): arbitrary character/word timings, gradient
  reveal, grouped lift/scale emphasis, and playback-clock integration.
- [Color flow](docs/color_flow.md): image-derived flowing light and brightness configuration.
- [Subtree effects](docs/subtree_effect.md): blur, wave, color adjustment and Bloom for element subtrees.
- [Subtree transitions](docs/subtree_transition.md): blur fades, crossfades and soft wipes between two UI subtrees.
- [History feedback](docs/feedback.md): persistent trails, time-based decay and playback controls.
- [Motion blur](docs/motion_blur.md): velocity-driven directional blur for moving subtrees.
- [Depth parallax](docs/depth_parallax.md): pointer-driven image depth with paired depth maps.
- [Water ripple](docs/ripple.md): local radial refraction for text and images.
- [Local lens](docs/lens.md): smooth local magnification and compression.
- [Interaction mapping](docs/interaction_mapping.md): pointer hit testing and dragging in deformed content.
- [Displacement maps](docs/displacement_map.md): external RG maps, local masks and texture-driven distortion.
- [GPU particles](docs/particles.md): light points, streaks, interactive forces and alpha-mask emission.
- [Particle transition](docs/particle_transition.md): reversible scattering and gathering of text and images.
- [GPU fluid](docs/fluid.md): interactive colored ink, momentum and vortices.
- [SDF shapes](docs/sdf.md): Boolean geometry, smooth blending, outlines and edge light.
- [Holographic material](docs/holographic.md): surface normals, directional lighting and foil reflections.
- [Contour light](docs/contour_glow.md): alpha-contour distance fields and edge-focused glow.
- [Contour relief](docs/contour_relief.md): raised and recessed bevels with directional lighting.
- [Contour shadow](docs/contour_shadow.md): directional soft shadows following text and image silhouettes.

## Local deformation

`subtree_deformation` applies a smooth local displacement to text, images and
other painted descendants. `DeformationOptions` controls the normalized capture
anchor, influence radius and translation. Translation is limited to 35% of the
radius to prevent folded content.

`ElasticOffset` supplies an independent, caller-clocked spring: hold a translation
with `drag_to`, call `release`, and advance the return with `advance`. Frequency
and damping are configurable through `spring`.

Use `EffectStage::deformation` to compose deformation with other subtree effects.
Layout remains unchanged. Enable `.map_interaction(true)` to align child pointer
targets with the deformation. Leave transparent space around the content for displaced edges.

## Examples

Run the frosted-glass example from the workspace root:

```sh
cargo run -p gpui_effects --example frosted_glass
```

Other examples in `examples/` demonstrate gradients, masked effects, motion
layers, and page-flip effects.

Run the timed-text example:

```sh
cargo run -p gpui_effects --example timed_text
```

Run the blurred-text example:

```sh
cargo run -p gpui_effects --example text_blur
```

Run the Bloom text and artwork comparison:

```sh
cargo run -p gpui_effects --example bloom
```

Run the two-content transition example:

```sh
cargo run -p gpui_effects --example subtree_transition
```

Run the history-feedback example:

```sh
cargo run -p gpui_effects --example feedback
```

Run the draggable motion-blur comparison:

```sh
cargo run -p gpui_effects --example motion_blur
```

Run the depth-map landscape example:

```sh
cargo run -p gpui_effects --example depth_parallax
```

Run the interactive water-ripple example:

```sh
cargo run -p gpui_effects --example ripple
```

Run the pointer-following lens example:

```sh
cargo run -p gpui_effects --example lens
```

Run the mapped button and slider example:

```sh
cargo run -p gpui_effects --example interaction_mapping
```

Run the interactive particle example:

```sh
cargo run -p gpui_effects --example particles
```

Run the text and artwork particle-emission example:

```sh
cargo run -p gpui_effects --example particle_mask
```

Run the reversible particle-transition example:

```sh
cargo run -p gpui_effects --example particle_transition
```

Run the displacement-map example:

```sh
cargo run -p gpui_effects --example displacement_map
```

Run the interactive fluid example:

```sh
cargo run -p gpui_effects --example fluid
```

Run the interactive shape-composition example:

```sh
cargo run -p gpui_effects --example sdf
```

Run the interactive foil-material example:

```sh
cargo run -p gpui_effects --example holographic
```

Run the draggable elastic-card example:

```sh
cargo run -p gpui_effects --example deformation
```

Run the text and icon contour-light comparison:

```sh
cargo run -p gpui_effects --example contour_glow
```

Run the pointer-lit relief comparison:

```sh
cargo run -p gpui_effects --example contour_relief
```

Run the pointer-lit contour-shadow example:

```sh
cargo run -p gpui_effects --example contour_shadow
```

## License

MIT. See [LICENSE](LICENSE).
