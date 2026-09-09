# gpui_3d TODO

Lightweight 3D scenes embedded in GUIs, with declarative construction and composition with GPUI layout, input, and effects.

Checked items are implemented. Unchecked items are planned, grouped by implementation phase.

## Implemented

- [x] Independent `gpui_3d` crate with a `Styled`-compatible `viewport3d` embedded in ordinary layouts.
- [x] Indexed triangle meshes, custom vertices, built-in planes and cubes, and shared geometry resources.
- [x] Object translation, Euler rotation, and nonuniform scaling with correct normal transforms.
- [x] Perspective cameras and orbital positioning, with drag-to-orbit and scroll-to-zoom in the example.
- [x] Depth testing, four-sample MSAA, double-sided rendering, and alpha cutout.
- [x] Solid, image, and decorative UI textures, basic directional and ambient lighting, and unlit materials.
- [x] Subtree composition, nested viewports, ancestor clipping, and group opacity.
- [x] Linux WGPU rendering, geometry caching, and intermediate target reuse.
- [x] Camera and normal math tests, scene replay tests, and offscreen rendering checks for depth, textures, clipping, nesting, and scaling.

## Phase 1: Objects and Interaction

- [x] **Geometric object picking**: Stable object IDs, screen rays, and nearest mesh hits returning world position, normal, triangle, and UV; expose hover and click callbacks.
- [x] **Picking example**: Hover highlighting, click selection, and drag-to-orbit with click/drag disambiguation, without changing object geometry.
- [x] **Image picking visibility**: Sample image alpha with material cutoffs, preserve depth order through cutouts, and skip images unavailable to the renderer.
- [x] **Picking behavior**: Selectable, occluder-only, and pass-through objects without changing rendering.
- [ ] **Captured UI picking visibility**: Sample capture alpha and keep hit results synchronized with captured content.
- [x] **UI texture sizing**: Independent logical layout dimensions and raster density, bounded texture allocation, DPI-aware glyph rendering, and a configurable example.
- [ ] **Multiple UI textures**: Attach independently sized UI sources to distinct scene materials.
- [x] **UI pointer mapping**: Route a named UI object's UVs into existing button, hover, scroll, and slider handlers, with gesture continuity outside the mesh and camera-input separation.
- [ ] **Focus and overlays**: Define ownership, positioning, and dismissal for keyboard focus, input methods, tooltips, and menus; specify unsupported interactions.
- [ ] **Camera controllers**: Reusable orbit, pan, and zoom with configurable targets, distance and angle limits, optional damping, and input conflict handling.
- [ ] **Interaction example**: Object selection, highlighting, and interactive 3D UI panels covering occlusion, device scales, and viewport sizes.

## Phase 2: Scenes and Assets

- [ ] **Scene hierarchy**: Nodes, parent-child transforms, visibility, and stable identities, with group movement and hiding.
- [ ] **Camera extensions**: Orthographic projection, viewport rays, world-to-screen projection, and framing from object bounds.
- [ ] **Geometry primitives**: Spheres, cylinders, cones, and subdivided planes with segment configuration, bounds, and normal and tangent generation.
- [ ] **Texture sampling**: UV transforms, addressing and filtering modes, mipmaps, anisotropic filtering, and distinct handling of color and data textures.
- [ ] **Static glTF / GLB models**: Load nodes, meshes, indices, UVs, normals, images, and basic materials; report unsupported extensions and asset errors.
- [ ] **Asynchronous assets**: Background model and texture loading, shared caches, loading states, and release policies without blocking the UI thread.
- [ ] **Model viewer example**: Local model loading, automatic framing, viewpoint switching, and node and material inspection.

## Phase 3: Materials and Lighting

- [ ] **Transparent materials**: Separate Opaque, Mask, and Blend modes; define transparent object sorting, depth writes, and intersecting-surface limitations.
- [ ] **Color pipeline**: Define sRGB input, linear lighting, HDR intermediate results, exposure, and tone mapping consistently with GPUI composition.
- [ ] **PBR materials**: Base color, metallic, roughness, normal, occlusion, and emissive parameters with usable defaults.
- [ ] **Light types**: Multiple directional, point, and spot lights with intensity, range, and attenuation controls.
- [ ] **Directional shadows**: Shadow maps, soft shadows, bias, and quality settings to control self-shadowing artifacts and resource costs.
- [ ] **Environment lighting**: HDR environment maps and diffuse and specular IBL, with independent background and lighting controls.
- [ ] **Material presets**: Matte, metal, plastic, and emissive presets using a consistent configuration API.
- [ ] **Effect integration**: Compose Bloom and color grading through `gpui_effects`; define depth texture access and coordinate conventions for depth-dependent effects.

## Phase 4: Animation and Dynamic Content

- [ ] **Node animation**: Translation, rotation, and scale tracks with interpolation, looping, pause, and seeking; support quaternion rotation.
- [ ] **Model animation**: glTF animation clips, skeletal skinning, and morph targets with clip selection and playback.
- [ ] **Dynamic geometry**: Update vertex and instance data while reusing GPU buffers instead of rebuilding mesh resources each frame.
- [ ] **3D annotations**: Anchor ordinary GPUI labels to scene positions with configurable depth occlusion and viewport edge behavior.

## Phase 5: Performance and Platforms

- [ ] **Viewport-sized render targets**: Allocate color, depth, and MSAA targets to viewport bounds, with configurable resolution and sample count to limit GPU memory use.
- [ ] **On-demand updates**: Track scene, camera, and UI texture invalidation separately; reuse results when static, invisible, or paused without continuously requesting frames.
- [ ] **Culling and instancing**: Frustum culling, instanced rendering of shared geometry, and material batching with reproducible benchmarks.
- [ ] **Resource lifecycle**: Handle multiple viewports, resizing, device recovery, and cache eviction within resource budgets.
- [ ] **Platform coverage**: Add macOS and Windows 3D rendering support with consistent capability queries and unsupported-backend behavior.
- [ ] **Cross-platform validation**: Cover depth, transparency, texture colors, nested composition, input mapping, and high DPI; distinguish automated checks from manual visual confirmation.

## Near-Term Order

1. Multiple UI textures.
2. Captured UI picking visibility and pointer mapping.
3. Reusable camera controllers, scene hierarchy, and more geometry primitives.
4. Texture sampling, transparent materials, and static glTF / GLB loading.
5. Color pipeline, PBR, lights, and shadows.

Address performance, resource lifecycle, and platform validation alongside each feature.
