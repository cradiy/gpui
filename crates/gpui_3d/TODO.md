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
- [x] **Camera damping**: Optional exponential target following for orbit, pan, dolly, and zoom, with explicit time advancement, release settling, cancellation, and an on-demand animation signal.
- [ ] **Interaction example**: Object selection, highlighting, and interactive 3D UI panels covering occlusion, device scales, and viewport sizes.

## Phase 2: Scenes and Resource Primitives

- [x] **Scene hierarchy**: Group and mesh nodes, graph-scoped generational handles, unique application IDs, inherited visibility, atomic reparenting, and subtree removal.
- [x] **Affine transforms**: Quaternion TRS and full affine matrices, including shear and negative scale, with keep-local/keep-world reparenting and inverse-transpose normals.
- [x] **Static scene evaluation**: Camera-independent owned results, world and aggregate bounds, node identities in picking, and hierarchy editing in the scene example.
- [x] **Camera and light nodes**: Optional local camera/light properties, complete world-transform evaluation, explicit camera selection, inherited light visibility, subtree reuse, structured validation, and shared viewport/headless scene preparation without extra geometry or picking IDs.
- [x] **Subtree reuse**: Immutable local snapshots, atomic cross-graph instantiation, explicit application-ID remapping, source-to-instance handles, shared geometry/images, and independent node properties.
- [x] **Mesh resource interfaces**: Fallible geometry construction with structured errors, borrowed vertex/index inspection, counts, local bounds, and shared immutable storage.
- [x] **Normal generation**: Flat face normals with vertex splitting, area-weighted smoothing by source index, output-to-source mappings for external attributes, preserved triangle identities, explicit tangent invalidation, and degenerate/cancellation diagnostics.
- [x] **Renderer resource interfaces**: Public scene preparation with typed texture requests, ready/pending states, contextual resource failures, renderer-local atlas references, and owned frame/identity outputs for external resource managers.
- [x] **Camera extensions**: Orthographic projection, explicit up vectors, public view/projection matrices, viewport rays, world-to-screen projection, and framing from object bounds.
- [x] **Bounds projection and frustum queries**: Reusable camera clip-volume snapshots, conservative world-AABB candidates, clipped screen rectangles across camera planes, viewport offsets and lens shifts, and outward-rounded pixel extents without occlusion claims.
- [x] **World-ray queries**: Normalized arbitrary rays with camera-independent geometric intersection and stable node identities.
- [x] **Query filtering**: Per-query node/application identity predicates for screen picks and world rays, BVH candidate pruning before mesh traversal, explicit exclusion/occlusion semantics, and unchanged render state and retained snapshots.
- [x] **Bounds overlap and distance**: Closed AABB intersections and finite f64 distances, BVH-pruned world-volume candidates with stable identities and ordered filtering, final-snapshot geometry, and explicit conservative rather than exact-mesh semantics.
- [x] **Frame coverage queries**: Single-pass CPU object-ID counts, fractions of the physical output, exclusive pixel bounds, background and zero-count records, retained camera/identity snapshots, and structured malformed-data errors with explicit sampling and transparency semantics.
- [x] **Mesh query acceleration**: Shared lazy CPU BVHs, explicit preparation, conservative transformed bounds, and original triangle identities for picking and world rays.
- [x] **Scene query acceleration**: Lazy object BVHs shared by scene clones and camera views of evaluated states, fresh indices after graph evaluation or object insertion, and preserved snapshot queries.
- [x] **Spatial index refitting**: Explicit previous-snapshot preparation, shared partitions with independent changed bounds, stable hidden-node slots, current-order query identities, topology-change rebuilds, and CPU build/refit benchmarks.
- [x] **Camera optics**: Focal-length/sensor-height conversion and explicit perspective/orthographic lens shift, with consistent matrices, rays, culling, background directions, framing, and camera controls.
- [x] **Projection extent**: Optional fixed aspect ratios and infinite-far perspective projection, shared by rendering, rays, linear-depth reconstruction, bounds/frustum queries and camera controls, with caller-owned output fitting.
- [ ] **Zero-near orthographic projection**: Camera-plane coverage with consistent world/screen queries and unambiguous linear-depth background semantics.
- [x] **Geometry primitives**: Configurable UV spheres, capped/open cylinders and cones, and subdivided XY planes; bounded generation, outward winding, split seams/tips/caps, analytic normal/tangent frames, mesh bounds, and shared query/render storage.
- [x] **Image sampling**: Affine UV transforms, independent Clamp/Repeat/Mirror addressing, Nearest/Linear filtering, atlas-local interpolation, and matching alpha-aware picking.
- [x] **Texture sampling extensions**: Independent linear-space image mip chains, area-weighted odd-size reduction, per-map nearest/trilinear level selection, 1–16× anisotropy, UV-gradient sampling, allocation-aware cache invalidation, and shared color/geometry-output alpha sampling. CPU ray alpha queries use level zero.
  - [x] **Independent texel filters**: Separate magnification overrides and minification filters for atlas and mipmapped images, sampler/cache identity, material batching, and explicit level-zero magnification filtering in CPU alpha queries.

## Phase 3: Materials and Lighting

- [x] **Material face visibility**: Explicit single/double-sided surfaces, reflection-aware local front faces, shared color/shadow/geometry-channel rejection, matching screen/world-ray queries, and independent material batching.
- [x] **Mask thresholds**: Finite nonnegative alpha cutoffs without clamping, including zero-alpha opaque coverage at zero and fully discarded masks above one, shared by rendering and queries.
- [x] **Transparent materials**: Opaque, Mask, and Blend modes; linear premultiplied blending, bounds-center object sorting, depth-write control, and alpha-aware picking/IDs. Intersecting and self-overlapping transparent surfaces require separate ordering solutions.
- [x] **Color pipeline**: sRGB and linear image inputs, linear filtering and lighting, RGBA16Float intermediate results, exposure, None/Reinhard tone mapping, and premultiplied display composition shared by viewport and headless rendering.
- [x] **PBR material factors**: Optional metallic-roughness shading, GGX/Smith/Schlick direct lighting, linear emissive radiance, and perspective/orthographic viewing directions shared by viewport and headless rendering.
- [x] **PBR factor textures**: Linear G/B metallic-roughness and sRGB emissive maps, factor multiplication, independent UV sampling, and shared viewport/headless resource resolution.
- [x] **Normal textures and tangent inputs**: Validated optional tangent data, built-in plane/cube tangents, linear RGB normal maps with independent sampling and XY strength, and reflection-aware world-space tangent frames.
- [x] **Tangent generation**: MikkTSpace face-corner frames, mirrored UV seam splitting, output-to-source vertex mapping for external attributes and deformation data, preserved triangle identities, and explicit degenerate/numerical input errors.
  - [x] **Degenerate tangent policies**: Strict validation, neighbor inheritance, and explicit derivative/orthonormal repairs with corner diagnostics, retained geometry, and consistent triangle handedness.
- [x] **Occlusion textures**: Linear R ambient occlusion maps, independent UV sampling and strength, indirect-only attenuation for basic and PBR materials, shared viewport/headless resource handling, and material controls.
- [x] **Light types**: Up to eight world-space directional, point, and spot lights; intensity, finite range, inverse-square distance clamping, soft cone controls, shared viewport/headless shading, and pointer-driven lighting controls.
- [x] **Directional shadows**: One selected directional source, explicit camera-independent coverage, 256–4096 shadow maps, PCF filtering, depth/normal bias, opaque/masked casting, per-mesh cast/receive controls, and shared viewport/headless shading.
- [x] **Environment lighting**: HDR environment maps and diffuse and specular IBL, with independent background and lighting controls.
  - [x] **Diffuse irradiance**: Decoded linear HDR equirectangular inputs, L2 spherical-harmonic projection, precomputed coefficient inputs, intensity and world-Y rotation, shared viewport/headless shading, and environment controls in the lighting example.
  - [x] **Specular IBL**: Explicit bounded GGX cube prefiltering, external prefiltered inputs, cached RGBA16Float cube levels and integrated BRDF lookup, normal-map-aware roughness-dependent reflections, independent intensity/rotation, and shared viewport/headless shading.
  - [x] **Environment background**: Shared decoded HDR maps, independent visibility/intensity/world-Y rotation, camera-correct distant rays, linear composition, cached uploads, and shared viewport/headless rendering without geometry-channel coverage.
- [x] **Effect integration**: Compose Bloom and color grading through `gpui_effects`; define depth texture access and coordinate conventions for depth-dependent effects.
  - [x] **Display-space viewport effects**: Bloom and color adjustment in one subtree chain, identity pointer mapping, independent controls in the lighting example, and existing viewport layout/clipping semantics.
  - [x] **Depth coordinates**: Retained output-camera snapshots, linear-depth world reconstruction for perspective/orthographic projections and lens shifts, physical pixel-center queries, and direct GPU sampling conventions.
  - [x] **GPU channel composition**: Direct one/two/four-texture effect processing, caller-owned batch encoding, per-input alpha/filter contracts, owned HDR/data outputs, byte admission, device validation, depth fog and HDR display mapping without CPU readback.

## Phase 4: Animation and Dynamic Content

- [x] **Transform evaluation**: Immutable translation, XYZW quaternion rotation, and scale tracks; Step, Linear/shortest-arc SLERP, and cubic Hermite interpolation; absolute-time sampling, explicit base poses, nonmutating hierarchy overrides, and shared rendering/query snapshots.
- [x] **Weight evaluation**: Runtime-sized signed weight tracks, absolute-time Step/Linear/CubicSpline sampling, per-second derivatives, immutable shared keys, allocation-free transactional output sampling, and explicit Morph/Skin composition.
- [x] **Pose blending and masks**: Immutable stable-node local TRS collections, shortest-arc rotation blending, sparse ordered override layers, explicit per-node/default masks, validated affine outputs, transactional failures, and composition with absolute-time tracks and constraints.
- [x] **Additive pose layers**: Explicit reference-relative translation, local shortest-arc rotation deltas, multiplicative signed scale, sparse node masks, retained base ordering, transactional validation, and absolute-time composition with hierarchy constraints.
- [x] **Deformation**: CPU linear-blend skeletal skinning and morph targets from explicit pose/weight inputs, shared by rendering and queries.
  - [x] **Morph targets**: Shared dense position/normal/tangent deltas, validated signed weights, normalized direction blending, preserved handedness, and immutable CPU-evaluated meshes reused by rendering, bounds, and picking.
  - [x] **Skeletal skinning**: Shared inverse bind matrices, arbitrary per-vertex influences with validated normalized weights, mesh-local/world-space joint inputs, inverse-transpose normals, reflection-aware tangents, morph composition, and immutable deformed geometry for rendering and queries.
- [x] **Dynamic geometry**: Update vertex and instance data while reusing GPU buffers instead of rebuilding mesh resources each frame.
  - [x] **Fixed-topology vertex updates**: Validated immutable vertex snapshots sharing index storage; fresh bounds/query indices, node mesh replacement, and topology-compatible GPU vertex/index buffer reuse with concurrent snapshot preservation.
  - [x] **Instance streams**: Explicit object transforms, normal matrices, base tints, and output IDs in reusable WGPU instance buffers; adjacent compatible opaque/masked draws share geometry, while blended objects retain independent ordered draws.
- [ ] **Attachments and constraints**: Stateless transform dependencies and pose solvers composed with animation evaluation.
  - [x] **Follow transforms**: Stable target handles, full affine offsets, final-pose dependency evaluation, cycle/reference diagnostics, independent visibility inheritance, preserved hierarchy/output ordering, and explicit release poses.
  - [x] **Aim constraints**: Stateless world-point solving and graph target offsets, explicit local forward/up axes and world up, affine-shape preservation, total-rotation limits with retained outcomes, and cycle/degeneracy diagnostics.
  - [x] **Two-bone IK**: Independent three-joint world-pose solving, target/pole inputs, rotation blending with preserved bone lengths, radial reach diagnostics, complete folding, and graph-local pose conversion.

## Phase 5: Performance and Platforms

- [x] **Direct headless 3D rendering**: Solid and decoded-image scenes without native windows or UI layout; shared viewport preparation and mesh passes, owned GPU outputs, and bounded nonblocking readback.
- [x] **Color and object ID outputs**: Display-encoded RGBA8 and exact R32Uint IDs, explicit pixel-center/MSAA coverage, zero ID background, and retained node/application identity maps.
- [x] **Depth and normal outputs**: Selectable R32Float camera-forward depth and Rgba32Float world-space vertex normals, pixel-center nearest-surface coverage, owned GPU textures, and bounded floating-point readback. Normal maps do not perturb geometry channels.
- [x] **HDR output**: Independently selectable premultiplied linear RGBA16Float textures, linear MSAA resolve, owned frames, and f32 readback before exposure and display mapping.
- [ ] **Capabilities and diagnostics**: Query formats, channels, sample counts, limits, and backend support; return actionable asset and rendering failures.
  - [x] **WGPU device reports**: Adapter/device feature separation, format usages and flags, output/channel/sample queries, enabled limits, image anisotropy, and pre-construction mesh-pipeline validation.
  - [x] **Viewport backend reports**: Live window support with backend/device/resource reasons, WGPU target-format validation independent of direct outputs, selected 1x/4x sampling, and device-bounded UI capture density.
- [x] **Viewport-sized render targets**: Viewport-local HDR color, depth, and MSAA attachments with configurable resolution and sample count; generic subtree-composition textures remain surface-sized.
  - [x] **Local mesh attachments**: Surface-clipped pixel bounds, preserved fractional alignment and UI sampling, size-shared HDR/depth/MSAA targets, active-size eviction, and surface-space composition.
  - [x] **Viewport quality controls**: Positive finite resolution scale, one/four color samples with capability fallback, mixed-quality viewport pipelines, device-bounded attachments, bilinear reconstruction, and unchanged input/UI texture coordinates.
- [ ] **On-demand updates**: Track scene, camera, and UI texture invalidation separately; reuse results when static, invisible, or paused without continuously requesting frames.
  - [x] **Retained CPU preparation**: Single-entry scene/snapshot reuse, camera/aspect/UI configuration invalidation, per-call active resource refresh, immutable prepared outputs, and viewport/headless-local ownership.
  - [x] **Submitted viewport outputs**: Immutable frame identity, captured paint content, referenced atlas generations, submission-gated pixel reuse, bounded viewport-local output storage, dynamic-input bypass, and renderer-owned external submission.
  - [x] **UI capture reuse**: Retain static capture pixels independently of mesh output, compare newly painted content after unrelated UI updates, and isolate caller-owned pending writes.
  - [x] **Retained draw plans**: Immutable object snapshots, camera/shadow clip matrices, output mode and batch-limit keys; active-plan eviction and shared geometry-channel plans.
- [ ] **Culling and instancing**: Frustum culling, instanced rendering of shared geometry, and material batching with reproducible benchmarks.
  - [x] **Instanced material batches**: Shared-mesh, compatible-material batching for color, shadow, and geometry outputs without reordering depth writers.
  - [x] **Frustum culling**: Cached indexed mesh bounds, perspective/orthographic clip-volume tests with full object transforms, independent camera/shadow eligibility, resource-resolution pruning, and per-frame draw plans without renumbering IDs or changing world-ray queries.
  - [ ] **Rendering benchmarks**: Reproducible shared-mesh and mixed-material workloads with CPU preparation and draw-count measurements.
    - [x] **Scene preparation**: CPU-only shared geometry, mixed PBR, off-camera, and pending-image workloads at 1,024 and 16,384 objects.
    - [x] **Draw planning**: CPU-only batching workloads, per-pass mesh/instance/triangle counts, parameter-upload payload sizes, and retained submission statistics.
    - [x] **Draw encoding**: Opt-in serialized GPU submissions for shared geometry, mixed PBR, culled instances, and vertex updates; color and geometry-output passes with CPU timing and submitted draw-count checks.
    - [ ] **GPU measurements**: Run encoding workloads on supported hardware and record adapter-specific results separately from CPU planning benchmarks.
- [ ] **Resource lifecycle**: Handle multiple viewports, resizing, device recovery, and cache eviction within resource budgets.
  - [x] **Explicit cache release**: Window-local mesh cache release preserving shared 2D resources; direct-renderer cache release preserving public atlas tiles; headless cache and private atlas release with retained frame/readback ownership and lazy reconstruction.
  - [x] **Mesh output-cache budgets**: Configurable window/external-renderer quotas shared by nested UI captures, allocation counts and bytes, immediate entry release on budget changes, zero-budget bypass, and recovery-preserved settings.
  - [x] **Instance capacity reclamation**: Device-bounded growth, quarter-capacity shrink hysteresis, active-batch reuse, and removed-batch release across viewport and direct-output passes.
  - [x] **Direct target admission**: CPU-only output/attachment/shadow payload reports, optional per-request byte limits before uploads, preserved cache/frame ownership, and submission-local reports independent of cache hits.
- [ ] **Platform coverage**: Add macOS and Windows 3D rendering support with consistent capability queries and unsupported-backend behavior.
- [ ] **Cross-platform validation**: Cover depth, transparency, texture colors, nested composition, input mapping, and high DPI; distinguish automated checks from manual visual confirmation.

## Extensions Built on the Core

- [ ] **glTF / GLB importer**: A separate `gpui_3d_gltf` crate translating file nodes, primitives, materials, and images into core data, with explicit unsupported-feature errors.
  - [x] **Document resources**: Bounded JSON/GLB parsing, caller-owned URI resolution, shared binary/image payloads, checked accessor layouts and sparse indices, encoded-byte admission, and retryable preparation without I/O or GPU policy.
  - [x] **Primitive geometry**: Indexed/non-indexed triangles, strips and fans, interleaved/sparse/zero-initialized accessors, selected normalized UV sets, flat normal and MikkTSpace tangent generation, source-vertex mappings, and bounded conversion into core meshes with explicit unsupported-attribute errors.
  - [x] **Material conversion**: PBR factors, alpha modes, face visibility, five image slots, per-slot color/sampling semantics, KHR texture transforms and Unlit, retained encoded inputs with caller-owned decoding, shared image resolution, and explicit UV/filter compatibility checks.
  - [x] **Static mesh scenes**: Selected-scene hierarchy conversion, source node/primitive mappings, shared geometry and scene-wide image resolution, aggregate conversion limits, transferable CPU definitions, and reusable core subtrees with independent instance properties.
  - [x] **Image decoding**: PNG/JPEG to straight-alpha BGRA8, signature/MIME validation, dimension and pixel admission, per-call aggregate output budgets, per-image working admission, shared active-image resolution, and caller-owned scheduling with custom decoder support.
  - [x] **Camera conversion**: Local perspective/orthographic cameras, authored aspect and clip parameters, infinite perspective far planes, hierarchy attachment, source-camera mappings and explicit per-instance selection; positive near depth is required.
  - [x] **Animation tracks**: Document-local node bindings, absolute-time TRS and Morph weight channels, Step/Linear/CubicSpline conversion, normalized integer rotation/weight inputs, authored base poses, bounded CPU definitions, and explicit caller-owned instance mapping.
  - [x] **Skeletal skins**: Consecutive joint/weight sets, generated-vertex correspondence, ordered inverse binds, shared primitive bindings, selected-scene hierarchy validation, authored initial deformation, and explicit per-instance final-pose skinning with aggregate admission.
  - [x] **Morph geometry**: Position/normal/tangent deltas, sparse inputs, authored weights, fixed generated-vertex correspondence, direction regeneration, and per-instance Morph-before-Skin evaluation with bounded shared inputs and initial output admission.
  - [x] **CPU asset inspection**: Local glTF/GLB conversion, image decoding, absolute-time scene deformation, repeated-time fingerprints, world bounds, and restricted relative-file resolution in one command-line example.
  - [ ] **Asset compatibility**:
    - [x] **Degenerate-UV tangents**: Reported base-mesh repairs, matching Morph evaluation policy, preserved primitive/vertex mappings, and CPU inspection diagnostics.
    - [ ] **Multiple active UV sets**: Independent material-slot coordinate sets shared by rendering, tangent generation, and alpha queries.
    - [ ] **Vertex colors**: Linear color/alpha attributes shared by material shading, deformation mappings, and geometry-output visibility.
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
