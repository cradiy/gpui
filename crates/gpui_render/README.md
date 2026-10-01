# GPUI render contracts

`gpui_render` owns the shared WGSL definitions and shaders for rectangles,
background fills, rounded corners, borders, shadows, underlines, image and text
sprites, RGBA/NV12 surfaces, and path rasterization and composition. WGPU uses
these definitions directly. The build script generates standalone MSL and HLSL
primitive shaders for the native Metal and Direct3D renderers.

`PrimitiveGlobals` defines the shared uniform layout. Instances retain their
GPUI primitive layouts. Native renderers bind
their instance slices and uniforms to the generated shader interface; device
creation, resource lifetimes, command submission and presentation remain backend
responsibilities.

Native shaders consume and emit sRGB-encoded colors, preserving the
selected gradient interpolation space. WGPU uses its linear-color conventions
and selects straight or premultiplied output through the uniforms.

Path rasterization writes premultiplied colors to a resolved intermediate
texture. Composition reads the corresponding physical pixel directly; both
passes use the same viewport dimensions and origin.

Color sprites use hardware linear filtering with atlas tile clamping. The
Direct3D 11 generator maps the single fixed sampler to register `s0`; unexpected
sampler interfaces fail generation instead of producing incompatible bindings.

Grayscale and subpixel text share coverage correction through `GammaParams`.
Platform font settings supply contrast, gamma and RGB/BGR order; zero correction
parameters retain the rasterizer's coverage. Subpixel output requires dual-source
blending. Metal uses grayscale coverage for text.

Surface shaders sample the visible UV region, apply the supplied YUV conversion
matrix, and compose opacity, rounded corners and content masks. Frame import,
texture upload and stream caching are backend responsibilities.

Effect and backdrop instances use GPUI's shared GPU layouts. WGPU and Direct3D
share the separable blur shader, with hardware linear sampling on WGPU and manual
bilinear sampling on Direct3D. Metal uses Metal Performance Shaders for blur.

The `native-shaders` feature provides dynamic effect translation to MSL and HLSL.
Its resource contracts cover image effects, masks, backdrop effects and blur
passes. Backdrop composition selects hardware or manual sampling explicitly.
`cargo test -p gpui_render --features native-shaders` checks both translations
without requiring a native graphics device.

`cargo test -p gpui_render` checks the host layout and shader resource contract.
MSL and HLSL generation is validated on every build. Native shader compilation
uses the platform SDK: Metal on macOS and FXC on Windows.
