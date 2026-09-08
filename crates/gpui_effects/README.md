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
- [History feedback](docs/feedback.md): persistent trails, time-based decay and playback controls.
- [Water ripple](docs/ripple.md): local radial refraction for text and images.
- [Local lens](docs/lens.md): smooth local magnification and compression.
- [GPU particles](docs/particles.md): GPU-simulated light points, streaks and interactive forces.
- [GPU fluid](docs/fluid.md): interactive colored ink, momentum and vortices.
- [SDF shapes](docs/sdf.md): Boolean geometry, smooth blending, outlines and edge light.
- [Path motion](docs/path_motion.md): stroke reveal, arc-length sampling and flowing dashes.
- [Holographic material](docs/holographic.md): surface normals, directional lighting and foil reflections.

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

Run the history-feedback example:

```sh
cargo run -p gpui_effects --example feedback
```

Run the interactive water-ripple example:

```sh
cargo run -p gpui_effects --example ripple
```

Run the pointer-following lens example:

```sh
cargo run -p gpui_effects --example lens
```

Run the interactive particle example:

```sh
cargo run -p gpui_effects --example particles
```

Run the interactive fluid example:

```sh
cargo run -p gpui_effects --example fluid
```

Run the interactive shape-composition example:

```sh
cargo run -p gpui_effects --example sdf
```

Run the vector-path animation example:

```sh
cargo run -p gpui_effects --example path_motion
```

Run the interactive foil-material example:

```sh
cargo run -p gpui_effects --example holographic
```

## License

MIT. See [LICENSE](LICENSE).
