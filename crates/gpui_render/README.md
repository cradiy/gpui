# GPUI render contracts

`gpui_render` owns the shared WGSL definitions and shaders for rectangles,
background fills, rounded corners, borders, shadows, underlines and path
rasterization. WGPU composes these definitions with its other primitive shaders.
The build script generates
standalone MSL and HLSL primitive shaders for the native Metal and Direct3D renderers.

`PrimitiveGlobals` defines the shared uniform layout. Instances retain their
`gpui::Quad`, `gpui::Shadow` and `gpui::Underline` layouts. Native renderers bind
their instance slices and uniforms to the generated shader interface; device
creation, resource lifetimes, command submission and presentation remain backend
responsibilities.

Native shaders consume and emit sRGB-encoded colors, preserving the
selected gradient interpolation space. WGPU uses its linear-color conventions
and selects straight or premultiplied output through the uniforms. Other native
primitives retain their backend shaders.

`cargo test -p gpui_render` checks the host layout and shader resource contract.
MSL and HLSL generation is validated on every build. Native shader compilation
uses the platform SDK: Metal on macOS and FXC on Windows.
