# gpui_3d TODO

Low-level 3D scene structures, resource references, evaluation, queries, and rendering, with viewports composed through GPUI layout, input, and effects.

The core is format-independent. File importers, asset catalogs, model-management wrappers, and viewer applications build on its public APIs as separate extensions. Core APIs expose data and rendering mechanisms without owning project files, import workflows, or application policy.

Checked items are implemented. Unchecked items are planned, grouped by implementation phase.

## Implemented

- [x] Independent `gpui_3d` crate with a `Styled`-compatible `viewport3d` embedded in ordinary layouts.
- [x] Indexed triangle meshes, custom vertices, built-in planes and cubes, and shared geometry resources.
- [x] Object translation, Euler rotation, and nonuniform scaling with correct normal transforms.
- [x] Perspective and orthographic cameras, public projection/ray queries, bounds framing, and interactive camera examples.
- [x] Depth testing, four-sample MSAA, double-sided rendering, and alpha cutout.
- [x] Solid, image, and decorative UI textures, basic directional and ambient lighting, and unlit materials.
- [x] Subtree composition, nested viewports, ancestor clipping, and group opacity.
- [x] Linux WGPU rendering, geometry caching, and intermediate target reuse.
- [x] Camera and normal math tests, scene replay tests, and offscreen rendering checks for depth, textures, clipping, nesting, and scaling.

## Phase 1: Objects and Interaction

- [x] **Geometric object picking**: Stable object IDs, screen rays, and nearest mesh hits returning world position, normal, triangle, and UV; expose hover and click callbacks.
- [x] **Scene interaction example**: Hover feedback, click selection, subtree edits, projection and framing controls, with separate orbit/pan gestures and unchanged geometry during selection.
- [x] **Image picking visibility**: Sample image alpha with material cutoffs, preserve depth order through cutouts, and skip images unavailable to the renderer.
- [x] **Picking behavior**: Selectable, occluder-only, and pass-through objects without changing rendering.
- [ ] **Captured UI picking visibility**: Sample capture alpha and keep hit results synchronized with captured content.
- [x] **UI texture sizing**: Independent logical layout dimensions and raster density, bounded texture allocation, DPI-aware glyph rendering, and a configurable example.
- [ ] **Multiple UI textures**: Attach independently sized UI sources to distinct scene materials.
- [x] **UI pointer mapping**: Route a named UI object's UVs into existing button, hover, scroll, and slider handlers, with gesture continuity outside the mesh and camera-input separation.
- [ ] **Focus and overlays**: Define ownership, positioning, and dismissal for keyboard focus, input methods, tooltips, and menus; specify unsupported interactions.
- [x] **Camera controllers**: Immediate orbit, target-plane pan, dolly, and optical zoom with configurable bindings, speeds, distance/pitch/optical limits, and gesture ownership.
- [ ] **Camera damping**: Optional inertia and damping with explicit time advancement and on-demand redraw scheduling.
- [ ] **Interaction example**: Object selection, highlighting, and interactive 3D UI panels covering occlusion, device scales, and viewport sizes.

## Phase 2: Scenes and Resource Primitives

- [x] **Scene hierarchy**: Group and mesh nodes, graph-scoped generational handles, unique application IDs, inherited visibility, atomic reparenting, and subtree removal.
- [x] **Affine transforms**: Quaternion TRS and full affine matrices, including shear and negative scale, with keep-local/keep-world reparenting and inverse-transpose normals.
- [x] **Static scene evaluation**: Camera-independent owned results, world and aggregate bounds, node identities in picking, and hierarchy editing in the scene example.
- [x] **Camera and light nodes**: Optional local camera/light properties, complete world-transform evaluation, explicit camera selection, inherited light visibility, subtree reuse, structured validation, and shared viewport/headless scene preparation without extra geometry or picking IDs.
- [x] **Subtree reuse**: Immutable local snapshots, atomic cross-graph instantiation, explicit application-ID remapping, source-to-instance handles, shared geometry/images, and independent node properties.
- [x] **Mesh resource interfaces**: Fallible geometry construction with structured errors, borrowed vertex/index inspection, counts, local bounds, and shared immutable storage.
- [x] **Renderer resource interfaces**: Public scene preparation with typed texture requests, ready/pending states, contextual resource failures, renderer-local atlas references, and owned frame/identity outputs for external resource managers.
- [x] **Camera extensions**: Orthographic projection, explicit up vectors, public view/projection matrices, viewport rays, world-to-screen projection, and framing from object bounds.
- [x] **World-ray queries**: Normalized arbitrary rays with camera-independent geometric intersection and stable node identities.
- [x] **Mesh query acceleration**: Shared lazy CPU BVHs, explicit preparation, conservative transformed bounds, and original triangle identities for picking and world rays.
- [x] **Scene query acceleration**: Lazy object BVHs shared by scene clones and camera views of evaluated states, fresh indices after graph evaluation or object insertion, and preserved snapshot queries.
- [ ] **Spatial index refitting**: Incremental object-bound updates for changed transforms and visibility without full index reconstruction.
- [x] **Camera optics**: Focal-length/sensor-height conversion and explicit perspective/orthographic lens shift, with consistent matrices, rays, culling, background directions, framing, and camera controls.
- [x] **Geometry primitives**: Configurable UV spheres, capped/open cylinders and cones, and subdivided XY planes; bounded generation, outward winding, split seams/tips/caps, analytic normal/tangent frames, mesh bounds, and shared query/render storage.
- [x] **Image sampling**: Affine UV transforms, independent Clamp/Repeat/Mirror addressing, Nearest/Linear filtering, atlas-local interpolation, and matching alpha-aware picking.
- [ ] **Texture sampling extensions**: Mipmaps and anisotropic filtering.

## Phase 3: Materials and Lighting

- [x] **Transparent materials**: Opaque, Mask, and Blend modes; linear premultiplied blending, bounds-center object sorting, depth-write control, and alpha-aware picking/IDs. Intersecting and self-overlapping transparent surfaces require separate ordering solutions.
- [x] **Color pipeline**: sRGB and linear image inputs, linear filtering and lighting, RGBA16Float intermediate results, exposure, None/Reinhard tone mapping, and premultiplied display composition shared by viewport and headless rendering.
- [x] **PBR material factors**: Optional metallic-roughness shading, GGX/Smith/Schlick direct lighting, linear emissive radiance, and perspective/orthographic viewing directions shared by viewport and headless rendering.
- [x] **PBR factor textures**: Linear G/B metallic-roughness and sRGB emissive maps, factor multiplication, independent UV sampling, and shared viewport/headless resource resolution.
- [x] **Normal textures and tangent inputs**: Validated optional tangent data, built-in plane/cube tangents, linear RGB normal maps with independent sampling and XY strength, and reflection-aware world-space tangent frames.
- [ ] **Tangent generation**: MikkTSpace-compatible tangent generation with UV seam splitting and explicit handling of degenerate inputs.
- [x] **Occlusion textures**: Linear R ambient occlusion maps, independent UV sampling and strength, indirect-only attenuation for basic and PBR materials, shared viewport/headless resource handling, and material controls.
- [x] **Light types**: Up to eight world-space directional, point, and spot lights; intensity, finite range, inverse-square distance clamping, soft cone controls, shared viewport/headless shading, and pointer-driven lighting controls.
- [x] **Directional shadows**: One selected directional source, explicit camera-independent coverage, 256–4096 shadow maps, PCF filtering, depth/normal bias, opaque/masked casting, per-mesh cast/receive controls, and shared viewport/headless shading.
- [x] **Environment lighting**: HDR environment maps and diffuse and specular IBL, with independent background and lighting controls.
  - [x] **Diffuse irradiance**: Decoded linear HDR equirectangular inputs, L2 spherical-harmonic projection, precomputed coefficient inputs, intensity and world-Y rotation, shared viewport/headless shading, and environment controls in the lighting example.
  - [x] **Specular IBL**: Explicit bounded GGX cube prefiltering, external prefiltered inputs, cached RGBA16Float cube levels and integrated BRDF lookup, normal-map-aware roughness-dependent reflections, independent intensity/rotation, and shared viewport/headless shading.
  - [x] **Environment background**: Shared decoded HDR maps, independent visibility/intensity/world-Y rotation, camera-correct distant rays, linear composition, cached uploads, and shared viewport/headless rendering without geometry-channel coverage.
- [ ] **Effect integration**: Compose Bloom and color grading through `gpui_effects`; define depth texture access and coordinate conventions for depth-dependent effects.

## Phase 4: Animation and Dynamic Content

- [x] **Transform evaluation**: Immutable translation, XYZW quaternion rotation, and scale tracks; Step, Linear/shortest-arc SLERP, and cubic Hermite interpolation; absolute-time sampling, explicit base poses, nonmutating hierarchy overrides, and shared rendering/query snapshots.
- [x] **Deformation**: CPU linear-blend skeletal skinning and morph targets from explicit pose/weight inputs, shared by rendering and queries.
  - [x] **Morph targets**: Shared dense position/normal/tangent deltas, validated signed weights, normalized direction blending, preserved handedness, and immutable CPU-evaluated meshes reused by rendering, bounds, and picking.
  - [x] **Skeletal skinning**: Shared inverse bind matrices, arbitrary per-vertex influences with validated normalized weights, mesh-local/world-space joint inputs, inverse-transpose normals, reflection-aware tangents, morph composition, and immutable deformed geometry for rendering and queries.
- [x] **Dynamic geometry**: Update vertex and instance data while reusing GPU buffers instead of rebuilding mesh resources each frame.
  - [x] **Fixed-topology vertex updates**: Validated immutable vertex snapshots sharing index storage; fresh bounds/query indices, node mesh replacement, and topology-compatible GPU vertex/index buffer reuse with concurrent snapshot preservation.
  - [x] **Instance streams**: Explicit object transforms, normal matrices, base tints, and output IDs in reusable WGPU instance buffers; adjacent compatible opaque/masked draws share geometry, while blended objects retain independent ordered draws.

## Phase 5: Performance and Platforms

- [x] **Direct headless 3D rendering**: Solid and decoded-image scenes without native windows or UI layout; shared viewport preparation and mesh passes, owned GPU outputs, and bounded nonblocking readback.
- [x] **Color and object ID outputs**: Display-encoded RGBA8 and exact R32Uint IDs, explicit pixel-center/MSAA coverage, zero ID background, and retained node/application identity maps.
- [x] **Depth and normal outputs**: Selectable R32Float camera-forward depth and Rgba32Float world-space vertex normals, pixel-center nearest-surface coverage, owned GPU textures, and bounded floating-point readback. Normal maps do not perturb geometry channels.
- [x] **HDR output**: Independently selectable premultiplied linear RGBA16Float textures, linear MSAA resolve, owned frames, and f32 readback before exposure and display mapping.
- [ ] **Capabilities and diagnostics**: Query formats, channels, sample counts, limits, and backend support; return actionable asset and rendering failures.
- [ ] **Viewport-sized render targets**: Allocate color, depth, and MSAA targets to viewport bounds, with configurable resolution and sample count to limit GPU memory use.
- [ ] **On-demand updates**: Track scene, camera, and UI texture invalidation separately; reuse results when static, invisible, or paused without continuously requesting frames.
- [ ] **Culling and instancing**: Frustum culling, instanced rendering of shared geometry, and material batching with reproducible benchmarks.
  - [x] **Instanced material batches**: Shared-mesh, compatible-material batching for color, shadow, and geometry outputs without reordering depth writers.
  - [x] **Frustum culling**: Cached indexed mesh bounds, perspective/orthographic clip-volume tests with full object transforms, independent camera/shadow eligibility, resource-resolution pruning, and per-frame draw plans without renumbering IDs or changing world-ray queries.
  - [ ] **Rendering benchmarks**: Reproducible shared-mesh and mixed-material workloads with CPU preparation and draw-count measurements.
    - [x] **Scene preparation**: CPU-only shared geometry, mixed PBR, off-camera, and pending-image workloads at 1,024 and 16,384 objects.
    - [x] **Draw planning**: CPU-only batching workloads, per-pass mesh/instance/triangle counts, parameter-upload payload sizes, and retained submission statistics.
    - [ ] **Draw encoding**: GPU upload/encoding workloads.
- [ ] **Resource lifecycle**: Handle multiple viewports, resizing, device recovery, and cache eviction within resource budgets.
- [ ] **Platform coverage**: Add macOS and Windows 3D rendering support with consistent capability queries and unsupported-backend behavior.
- [ ] **Cross-platform validation**: Cover depth, transparency, texture colors, nested composition, input mapping, and high DPI; distinguish automated checks from manual visual confirmation.

## Extensions Built on the Core

- [ ] **glTF / GLB importer**: A separate `gpui_3d_gltf` crate translating file nodes, primitives, materials, and images into core data, with explicit unsupported-feature errors.
- [ ] **Model assets and instances**: Asset/instance/primitive ownership and instance-level overrides built on shared resources and subtree mappings.
- [ ] **Asynchronous asset management**: File resolution, decoding, background loading, caches, retries, and release policies outside the core renderer.
- [ ] **Model viewer**: Local model loading, automatic framing, viewpoint switching, and node/material inspection as an extension example or application.
- [ ] **Animation import and playback**: File-format clips, playback state, looping, pause, and seeking feeding core pose, transform, and deformation inputs.
- [ ] **Material presets**: Matte, metal, plastic, and emissive configurations built on core material parameters.
- [ ] **3D annotations**: GPUI label widgets built on projection and depth queries, with configurable visibility and edge behavior.

## Near-Term Core Order

1. Resource interfaces and accelerated spatial queries for external loaders and editors.
2. Texture sampling, linear HDR color, PBR, and transparent materials.
3. Depth/normal outputs, environment lighting, and directional shadows.
4. Explicit pose/deformation inputs and absolute-time evaluation, followed by attachments and constraints.

Keep multiple UI textures, capture-alpha picking, and focus/overlay support as independent GUI extensions.

Address performance, resource lifecycle, and platform validation alongside each feature.
