# gpui_effects

GPU-driven visual effects and reusable effect components for GPUI applications.
WGSL is the canonical shader implementation, with GPUI providing the render
pipeline and `gpui_effects` providing higher-level components and presets.

## Guides

- [Frosted glass](docs/glass.md): strongly blurred panels and mergeable rounded surfaces.
- [Timed text](docs/timed_text.md): arbitrary character/word timings, gradient
  reveal, grouped lift/scale emphasis, and playback-clock integration.

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

## License

MIT. See [LICENSE](LICENSE).
