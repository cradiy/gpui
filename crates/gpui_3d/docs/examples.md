# 3D examples

[Overview](../README.md) · [Viewport integration](viewport.md)

Run the examples from the repository root. Each is an independent executable.

| Example | Controls and content |
| --- | --- |
| [scene](../examples/scene.rs) | Hierarchy, shared instances, selection, cameras, animation, CPU/GPU Morph and Skin, and generated tangent frames. |
| [materials](../examples/materials.rs) | PBR, custom Toon/Sphere Map programs, outlines, texture sampling, and alpha modes. |
| [lighting](../examples/lighting.rs) | Direct lights, HDR environments, directional shadows, Bloom, and color adjustment. |
| [ui](../examples/ui.rs) | Captured UI buttons, slider and scrolling, occlusion, logical layout size, and raster density. |
| [headless](../examples/headless.rs) | Window-free display/HDR/ID/depth/normal readback, PNG previews, and object identities. |

```sh
cargo run -p gpui_3d --features wgpu --example scene
cargo run -p gpui_3d --features wgpu --example materials
cargo run -p gpui_3d --example lighting
cargo run -p gpui_3d --example ui
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

## Scene and deformation

In `scene`, hover an assembly to highlight its toolbar control, or click it to
select it. The numbered controls also select assemblies. Move or
tint its body, rotate the assembly, or hide its subtree. Other instances retain
their own properties. Right-drag to orbit, middle-drag to pan, and scroll to zoom.
Projection preserves the apparent size at the target; Frame selected fits the
selected assembly's bounds.
Camera damping toggles an 80 ms response half-life for manual camera controls.

Normal map applies a directional ridge pattern to the bodies. Regenerate tangents
derives their tangent frames from the deformed geometry before skinning. Combine
it with Blend shapes and Bend skin, then switch CPU/GPU deformation at a paused
time to compare shading. The GPU control requires the `wgpu` feature; tangent
regeneration also requires [device-enabled `SHADER_F64`](topics/tangent_weld.md#work-and-memory).
GPU tangent generation expands triangle corners and remaps Morph and Skin bindings; it keeps
the prepared initial pose or last validated output visible while the next result
is pending. Errors are displayed in the viewport without switching to CPU evaluation.

## Materials

In `materials`, the spheres share geometry and expose different material responses.
Normal and occlusion maps toggle independently of metallic-roughness and emissive
maps. The strip below the spheres shows image alpha over an opaque background.
Cycle Opaque/Mask/Blend, Clamp/Repeat/Mirror, and Nearest/Linear; density and offset
also affect the strip. Exposure and tone mapping apply to the complete 3D scene.

With `wgpu`, **PBR / Custom** compares the built-in response with Toon shading
and a camera-space Sphere Map. Uniform controls adjust the custom materials,
and outline controls exercise an additional mesh pass. The
[material program reference](topics/material_programs.md#material-comparison)
and [mesh pass reference](topics/mesh_passes.md#material-comparison) describe
the shader inputs and controls.

## Lighting

In `lighting`, move the pointer to steer the source. Shadow controls apply only
to the directional source. Lift the objects to inspect detached shadows, or toggle
the environment and fill light to inspect illumination inside shadowed areas.

## UI surfaces

In `ui`, drag the slider beyond the panel and scroll the notes. Toggle the occluder
to block part of the panel. Density changes raster quality without reflow; canvas
width changes layout and the mesh aspect ratio. Right-drag to orbit, or left-drag
empty space. Scroll outside the panel to zoom.

## Headless output

The `headless` example writes channel previews into the supplied directory and
reports object identities. See [headless output](topics/headless.md#example) for
file names, formats, and readback behavior.

For file-backed scenes, use the [glTF/GLB viewer and inspector](../../gpui_3d_gltf/README.md#try-a-model).
