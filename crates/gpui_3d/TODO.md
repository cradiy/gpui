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
  - [x] **Material batches**: Atomic multi-node material replacement with complete target validation, one revision increment, unchanged geometry and identities, and retained evaluated snapshots.
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
  - [x] **Point depth comparisons**: Constant-time world-point projection into retained depth frames, explicit background/front/tolerance/behind results, half-open pixel bounds, and typed input/channel errors without continuous-visibility claims.
  - [x] **CPU label images**: Bounded object-ID remapping to exact caller-defined u32 labels, merged primitive groups, zero exclusion, retained source identities, and composition with per-object coverage after readback.
- [x] **Rendered-frame picking**: Bounded one-pixel ID/depth readback, nonblocking completion, retained source camera/identity mapping, and world reconstruction independent of CPU geometry.
  - [ ] **Picking GPU validation**: Verify regional copy parity, GPU-deformed surface selection, background, and retained-frame ownership on supported adapters.
- [x] **Mesh query acceleration**: Shared lazy CPU BVHs, explicit preparation, conservative transformed bounds, and original triangle identities for picking and world rays.
- [x] **Scene query acceleration**: Lazy object BVHs shared by scene clones and camera views of evaluated states, fresh indices after graph evaluation or object insertion, and preserved snapshot queries.
- [x] **Spatial index refitting**: Explicit previous-snapshot preparation, shared partitions with independent changed bounds, stable hidden-node slots, current-order query identities, topology-change rebuilds, and CPU build/refit benchmarks.
- [x] **Camera optics**: Focal-length/sensor-height conversion and explicit perspective/orthographic lens shift, with consistent matrices, rays, culling, background directions, framing, and camera controls.
- [x] **Projection extent**: Optional fixed aspect ratios and infinite-far perspective projection, shared by rendering, rays, linear-depth reconstruction, bounds/frustum queries and camera controls, with caller-owned output fitting.
- [x] **Zero-near orthographic projection**: Camera-plane projection, reconstruction and picking, preserved glTF zero near depth, and frame-retained negative depth backgrounds distinct from zero-depth surfaces.
  - [ ] **Eye-plane GPU validation**: Verify raster coverage and depth readback at zero near depth on supported adapters.
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
    - [x] **Effect resource ownership**: Device-owned texture inputs and outputs with per-slot ownership admission before view creation and binding.
    - [ ] **Effect ownership GPU validation**: Verify foreign-device inputs in every binding slot, retained output/view identity, chained effects, and encoder reuse after input admission failures on supported adapters.

## Phase 4: Animation and Dynamic Content

- [x] **Transform evaluation**: Immutable translation, XYZW quaternion rotation, and scale tracks; Step, Linear/shortest-arc SLERP, and cubic Hermite interpolation; absolute-time sampling, explicit base poses, nonmutating hierarchy overrides, and shared rendering/query snapshots.
- [x] **Weight evaluation**: Runtime-sized signed weight tracks, absolute-time Step/Linear/CubicSpline sampling, per-second derivatives, immutable shared keys, allocation-free transactional output sampling, and explicit Morph/Skin composition.
  - [x] **Weight layers**: Immutable node-indexed arrays, sparse masked overrides, reference-relative additive mixing, component-count validation, transactional overflow failures, and direct imported-sample deformation inputs.
- [x] **Pose blending and masks**: Immutable stable-node local TRS collections, shortest-arc rotation blending, sparse ordered override layers, explicit per-node/default masks, validated affine outputs, transactional failures, and composition with absolute-time tracks and constraints.
- [x] **Additive pose layers**: Explicit reference-relative translation, local shortest-arc rotation deltas, multiplicative signed scale, sparse node masks, retained base ordering, transactional validation, and absolute-time composition with hierarchy constraints.
- [x] **Deformation**: CPU linear-blend skeletal skinning and morph targets from explicit pose/weight inputs, shared by rendering and queries.
  - [x] **Nonmutating mesh evaluation**: Combined local-transform and mesh overrides, preserved authored graphs and snapshots, replacement-aware bounds, spatial-index refits, and preparation-cache identity.
  - [x] **Final-pose mesh replacement**: Graph-independent geometry updates on evaluated snapshots, preserved world poses and constraint outcomes, replacement-aware visible/hidden bounds, fresh preparation identity, and spatial-index refits.
  - [x] **Constrained deformation**: Follow/Aim evaluation with replacement meshes, retained constraint outcomes, and shared final transforms for Morph/Skin geometry, bounds, queries, cameras, and lights.
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
- [x] **GPU label outputs**: Exact R32Uint remapping with bounded label uploads, independent textures, retained camera/source identities, and direct or caller-encoded submission without CPU pixel readback.
  - [x] **Texture ownership**: Device-owned output textures and label inputs, with foreign-device rejection before resource binding and assignment callbacks.
  - [ ] **Texture ownership GPU validation**: Verify output-channel identity retention, foreign-device rejection, callback admission, and label resource lifetime on supported adapters.
  - [ ] **GPU label validation**: Verify pixel parity with CPU labels, queue ordering, and retained outputs on supported adapters.
- [x] **Depth and normal outputs**: Selectable R32Float camera-forward depth and Rgba32Float world-space vertex normals, pixel-center nearest-surface coverage, owned GPU textures, and bounded floating-point readback. Normal maps do not perturb geometry channels.
- [x] **HDR output**: Independently selectable premultiplied linear RGBA16Float textures, linear MSAA resolve, owned frames, and f32 readback before exposure and display mapping.
- [ ] **Capabilities and diagnostics**: Query formats, channels, sample counts, limits, and backend support; return actionable asset and rendering failures.
  - [x] **WGPU device reports**: Adapter/device feature separation, format usages and flags, output/channel/sample queries, enabled limits, image anisotropy, and pre-construction mesh-pipeline validation.
  - [x] **Viewport backend reports**: Live window support with backend/device/resource reasons, WGPU target-format validation independent of direct outputs, selected 1x/4x sampling, and device-bounded UI capture density.
- [x] **Viewport-sized render targets**: Viewport-local HDR color, depth, and MSAA attachments with configurable resolution and sample count; generic subtree-composition textures remain surface-sized.
  - [x] **Local mesh attachments**: Surface-clipped pixel bounds, preserved fractional alignment and UI sampling, size-shared HDR/depth/MSAA targets, active-size eviction, and surface-space composition.
  - [x] **Viewport quality controls**: Positive finite resolution scale, one/four color samples with capability fallback, mixed-quality viewport pipelines, device-bounded attachments, bilinear reconstruction, and unchanged input/UI texture coordinates.
- [ ] **On-demand updates**: Track scene, camera, and UI texture invalidation separately; reuse results when static, invisible, or paused without continuously requesting frames.
  - [x] **Retained CPU preparation**: Configurable entry-bounded scene/snapshot reuse, least-recently-used eviction, camera/aspect/UI configuration keys, resource rebinding without geometry recalculation, immutable prepared outputs, and viewport/headless-local ownership.
  - [x] **Submitted viewport outputs**: Immutable frame identity, captured paint content, referenced atlas generations, submission-gated pixel reuse, bounded viewport-local output storage, dynamic-input bypass, and renderer-owned external submission.
  - [x] **UI capture reuse**: Retain static capture pixels independently of mesh output, compare newly painted content after unrelated UI updates, and isolate caller-owned pending writes.
  - [x] **Retained draw plans**: Immutable object snapshots, camera/shadow clip matrices, output mode and batch-limit keys; active-plan eviction and shared geometry-channel plans.
- [ ] **Culling and instancing**: Frustum culling, instanced rendering of shared geometry, and material batching with reproducible benchmarks.
  - [x] **Instanced material batches**: Shared-mesh, compatible-material batching for color, shadow, and geometry outputs without reordering depth writers.
  - [x] **Frustum culling**: Cached indexed mesh bounds, perspective/orthographic clip-volume tests with full object transforms, independent camera/shadow eligibility, resource-resolution pruning, and per-frame draw plans without renumbering IDs or changing world-ray queries.
  - [ ] **Rendering benchmarks**: Reproducible shared-mesh and mixed-material workloads with CPU preparation and draw-count measurements.
    - [x] **Scene preparation**: CPU-only shared geometry, mixed PBR, off-camera, pending-image, alternating-camera, and resource-rebinding workloads at 1,024 and 16,384 objects.
    - [x] **Draw planning**: CPU-only batching workloads, per-pass mesh/instance/triangle counts, parameter-upload payload sizes, and retained submission statistics.
    - [x] **Draw encoding**: Opt-in serialized GPU submissions for shared geometry, mixed PBR, culled instances, and vertex updates; color and geometry-output passes with CPU timing and submitted draw-count checks.
    - [ ] **GPU measurements**: Run encoding workloads on supported hardware and record adapter-specific results separately from CPU planning benchmarks.
- [ ] **Resource lifecycle**: Handle multiple viewports, resizing, device recovery, and cache eviction within resource budgets.
  - [x] **Explicit cache release**: Window-local mesh cache release preserving shared 2D resources; direct-renderer cache release preserving public atlas tiles; headless cache and private atlas release with retained frame/readback ownership and lazy reconstruction.
  - [x] **Headless image residency**: Configurable idle image-count and pixel-payload limits, preparation-recency eviction, active-image preservation, idle usage reports, and rollback of new allocations on preparation failure.
  - [x] **Headless image admission**: Per-preparation decoded-pixel budgets with identity deduplication, resident-input checks, active/idle payload reports, and allocation rollback on rejection.
  - [x] **Mesh output-cache budgets**: Configurable window/external-renderer quotas shared by nested UI captures, allocation counts and bytes, immediate entry release on budget changes, zero-budget bypass, and recovery-preserved settings.
  - [x] **Instance capacity reclamation**: Device-bounded growth, quarter-capacity shrink hysteresis, active-batch reuse, and removed-batch release across viewport and direct-output passes.
  - [x] **Mixed-quality geometry sharing**: Viewport-local vertex/index pools across one/four-sample pipelines, shared snapshot/UV keys, retired-consumer release before topology reuse, and independent per-view attachments.
  - [x] **Mixed-quality image sharing**: Common mip-chain and sampler caches across one/four-sample viewport pipelines, union-based image retention, and independent atlas-generation/color-space keys.
  - [x] **Sampler reclamation**: Active material-input retention across prepared views, independent non-mipmapped sampling, and release after sampling changes, map deactivation, or viewport removal.
  - [x] **Submitted vertex uploads**: Queue-submission-gated staging release, unchanged-geometry copy reuse, shared geometry-channel ownership, and replay after failed or caller-owned encoding.
  - [x] **External vertex ownership**: Non-recyclable externally encoded vertex destinations, including initially populated buffers, with renderer-owned snapshot reuse and retained upload replay.
  - [x] **GPU packing source rebinding**: Same-device kernel reuse, shared index storage, independent base attribute uploads, complete-source admission, and retained prior sources/results across mesh replacement.
  - [x] **Direct target admission**: CPU-only output/attachment/shadow payload reports, optional per-request byte limits before uploads, preserved cache/frame ownership, and submission-local reports independent of cache hits.
  - [x] **Direct geometry admission**: CPU-only vertex/index payload reports with shared mesh/UV accounting, camera/shadow eligibility, per-buffer device checks, optional per-request totals before geometry allocation, and retained output reports.
  - [x] **Selective readback admission**: Available-channel subsets, aligned staging and widened CPU payload budgets before allocation, retained request reports, and direct decoding with the shared pending/cancellation limit.
  - [x] **Regional readback**: Checked physical pixel rectangles, region-sized channel budgets, copy origins and extents, and shared completion/cancellation behavior with full-frame reads.
- [ ] **Platform coverage**: Add macOS and Windows 3D rendering support with consistent capability queries and unsupported-backend behavior.
- [ ] **Cross-platform validation**: Cover depth, transparency, texture colors, nested composition, input mapping, and high DPI; distinguish automated checks from manual visual confirmation.

## Extensions Built on the Core

- [ ] **glTF / GLB importer**: A separate `gpui_3d_gltf` crate translating file nodes, primitives, materials, and images into core data, with explicit unsupported-feature errors.
  - [x] **Document resources**: Bounded JSON/GLB parsing, caller-owned URI resolution, shared binary/image payloads, checked accessor layouts and sparse indices, encoded-byte admission, and retryable preparation without I/O or GPU policy.
  - [x] **Import diagnostics**: Shared document-wide optional-extension advisories with JSON Pointer locations, opaque metadata boundaries, bounded record/text admission, explicit CLI inspection, and unchanged required-extension failures.
  - [x] **Primitive geometry**: Indexed/non-indexed triangles, strips and fans, interleaved/sparse/zero-initialized accessors, selected normalized UV sets, flat normal and MikkTSpace tangent generation, source-vertex mappings, and bounded conversion into core meshes with explicit unsupported-attribute errors.
  - [x] **Material conversion**: PBR factors, alpha modes, face visibility, five image slots, per-slot color/sampling semantics, KHR texture transforms and Unlit, retained encoded inputs with caller-owned decoding, shared image resolution, and explicit UV/filter compatibility checks.
  - [x] **Static mesh scenes**: Selected-scene hierarchy conversion, source node/primitive mappings, shared geometry and scene-wide image resolution, aggregate conversion limits, transferable CPU definitions, and reusable core subtrees with independent instance properties.
  - [x] **Image decoding**: PNG/JPEG to straight-alpha BGRA8, signature/MIME validation, dimension and pixel admission, per-call aggregate output budgets, per-image working admission, shared active-image resolution, and caller-owned scheduling with custom decoder support.
  - [x] **Camera conversion**: Local perspective/orthographic cameras, authored aspect and clip parameters, infinite perspective far planes, zero-near orthographic projection, hierarchy attachment, source-camera mappings and explicit per-instance selection.
  - [x] **Punctual lights**: KHR_lights_punctual directional, point and spot conversion, linear-color preservation, hierarchy attachment, source-light mappings, independent instance properties and selected-scene admission limits.
  - [x] **Animation tracks**: Document-local node bindings, absolute-time TRS and Morph weight channels, Step/Linear/CubicSpline conversion, normalized integer rotation/weight inputs, authored base poses, bounded CPU definitions, and explicit caller-owned instance mapping.
  - [x] **Skeletal skins**: Consecutive joint/weight sets, generated-vertex correspondence, ordered inverse binds, shared primitive bindings, selected-scene hierarchy validation, authored initial deformation, and explicit per-instance final-pose skinning with aggregate admission.
  - [x] **Morph geometry**: Position/normal/tangent deltas, sparse inputs, authored weights, fixed generated-vertex correspondence, direction regeneration, and per-instance Morph-before-Skin evaluation with bounded shared inputs and initial output admission.
  - [x] **CPU asset inspection**: Local glTF/GLB conversion, image decoding, absolute-time deformation, per-node Morph overrides, combined-binding counts, full vertex-attribute fingerprints, world bounds, and restricted relative-file resolution in one command-line example.
  - [ ] **Asset compatibility**:
    - [x] **Quantized geometry**: KHR_mesh_quantization integer positions, signed normal/tangent data, extended UV formats and signed Morph deltas, with normalized/interleaved/sparse decoding, declaration and alignment checks, preserved node/texture/skin dequantization, and decoded-element admission.
    - [x] **Degenerate-UV tangents**: Reported base-mesh repairs, matching Morph evaluation policy, preserved primitive/vertex mappings, and CPU inspection diagnostics.
    - [x] **Multiple active UV sets**: Independent material-slot coordinate sets shared by rendering, tangent generation, and alpha queries.
      - [x] **Mesh coordinate storage**: Validated sparse coordinate sets, immutable snapshots, and preservation through vertex splitting, Morph and Skin.
      - [x] **Selected-set tangents**: MikkTSpace and repair policies use the selected set, with preserved UV data, basis associations, and deformation propagation.
      - [x] **Coordinate selection**: Per-slot GPU coordinates and gradients, UV-aware geometry caching and batching, matching alpha coverage across outputs, and image-alpha queries.
      - [x] **glTF coordinate bindings**: Preserve authored coordinate sets, bind each material slot independently, retain selected tangent bases through Morph, and bound coordinate admission.
    - [x] **Vertex colors**: Normalized linear RGBA storage, split/deformation correspondence, material modulation, shared output and query alpha coverage, and glTF COLOR_0 conversion.
- [ ] **Model assets and instances**: Asset/instance/primitive ownership and instance-level overrides built on shared resources and subtree mappings.
  - [x] **glTF instance identities**: Retained shared assets, source-index and reverse occurrence mappings, authored material groups for graph overrides, application ID admission, and bound Morph/Skin evaluation.
  - [x] **Material edit sources**: Borrowed current node surfaces and shared resolved source-material lookup by authored index, retaining decoded images for property edits and batch restoration without replacing geometry.
- [ ] **Asynchronous asset management**: File resolution, decoding, background loading, caches, retries, and release policies outside the core renderer.
  - [x] **Asynchronous URI preparation**: Executor-independent glTF loaders, remaining-byte request budgets, shared synchronous/asynchronous validation, drop-based cancellation, and retryable partial-resource cleanup.
  - [x] **Scene load publication**: Per-slot request identity, supersession and cancellation, retained successful assets, transferable decoded resources, owner-thread resolution, and rejection of stale or foreign completions.
  - [x] **Encoded resource retention**: Caller-keyed shared payloads, byte/entry LRU limits, per-request admission on hits, explicit invalidation, in-flight insertion isolation, and consumer-preserving eviction.
  - [x] **Decoded image retention**: Content/MIME-keyed PNG/JPEG pixels, shared byte/entry LRU limits, per-call image admission and aggregate budgets on hits, worker transfer, and consumer-preserving release without retaining encoded buffers.
  - [x] **Bounded load admission**: Shared FIFO queues with independent active/waiting limits, lazy pipeline construction, typed saturation errors, cancellation-safe permit handoff, load-slot composition, and caller-owned execution.
- [ ] **Model viewer**: Local model loading, automatic framing, viewpoint switching, and node/material inspection as an extension example or application.
  - [x] **Local viewport example**: Background glTF/GLB loading, bounded queue and decoded-image reuse, cancel/reload with retained successful models, actual-viewport framing, Orbit and authored cameras with aspect fitting, and source node/material inspection.
- [ ] **Animation import and playback**: File-format clips, playback state, looping, pause, and seeking feeding core pose, transform, and deformation inputs.
  - [x] **Instance animation binding**: Source-identity validation, explicit missing-target policy, shared graph-handle mappings, and independent absolute-time Pose/Morph samples with retained results on failure.
  - [x] **Authored animation bases**: Instance-mapped TRS and per-node Morph defaults, retained independently of graph edits, with matrix and outer-placement preservation for sparse clip layers and crossfades.
  - [x] **Clip playback and viewer sampling**: Caller-advanced authored time ranges, signed rates, looping, pause/seek, partition-independent elapsed scaling, and selected-clip viewer controls with shared TRS/Morph/Skin snapshots and retained frames on sampling failure.
- [ ] **Material presets**: Matte, metal, plastic, and emissive configurations built on core material parameters.
- [ ] **3D annotations**: GPUI label widgets built on projection and depth queries, with configurable visibility and edge behavior.

## Skeletal Control and Simulation Integration

Core implementation order:

1. [x] **World-space pose inputs**: Mixed local/world transform overrides, parent-first evaluation independent of input order, unchanged authored graphs, stable identities, and final-pose geometry/query consistency.
2. [x] **Skin influence access**: Borrowed normalized per-vertex joint weights, explicit vertex-index errors, and immutable bindings suitable for weight inspection and external editing tools.
3. [x] **Joint rotation limits**: Stateless local-space swing/twist constraints with explicit reference frames, finite limits, and clamping diagnostics; independent of humanoid naming and physical joints.
4. [x] **IK extensions**: End-effector orientation and constrained multi-joint chains with explicit convergence/reach results and caller-owned time state.
   - [x] **End-effector orientation**: Independent terminal frame targets and weights, preserved positional solutions and affine shape, and rounded-output angular errors.
   - [x] **Constrained chains**: Multi-joint solving with local rotation limits, explicit convergence/reach diagnostics, and no internal playback history.
5. [x] **Deformation measurements**: Reproducible CPU Skin/Morph workloads covering mesh size, active influences, target counts, and concurrent instances; distinguish evaluation from upload/render time.
6. [ ] **GPU deformation**: Shared skin palettes and Morph inputs, bounded GPU buffers, retained outputs, and explicit CPU bounds/picking synchronization. Preserve CPU evaluation for callers requiring final geometry.
   - [x] **Capability preflight**: Per-operation compute checks shared with constructors, adapter/enabled limit reporting, and independent indirect-execution requirements for vertex packing.
   - [x] **GPU Morph**: Reusable uploaded targets, signed per-evaluation weights, retained attribute buffers, payload admission, and explicit CPU mesh readback.
   - [ ] **GPU validation**: Check Morph/Skin output parity, bounds reduction, composition, and retained-output lifetime on supported adapters.
   - [x] **GPU Skin**: Shared influence bindings, per-instance palettes, and Morph-to-Skin buffer composition.
   - [x] **Imported deformation inputs**: Shared glTF attribute targets, explicit direction-regeneration requirements, and instance-mapped final Skin poses in binding joint order.
   - [x] **GPU flat normals**: Retained triangle-corner topology, source identity and payload admission, and fixed-order reconstruction between Morph and Skin.
   - [x] **GPU smooth normals**: Retained indexed adjacency, ordered area weighting with widened arithmetic, fixed vertex correspondence, independent results and explicit degenerate/cancelled normal status.
   - [x] **GPU result remapping**: Retained output-to-source indices, bit-preserving record copies, duplication/reordering/subsets, destination topology identity and pre-allocation mapping/device admission.
   - [ ] **Remapping GPU validation**: Verify external snapshot reuse, exact record/status preservation, foreign devices, destination metadata and smooth-normal-to-corner-tangent composition on supported adapters.
   - [ ] **Smooth normal GPU validation**: Verify Morph/external inputs, area weighting, seams, unused vertices, extreme coordinate scales, failure propagation and retained packed outputs on supported adapters.
   - [ ] **GPU direction validation**: Verify normal reconstruction, degenerate-face status, retained outputs, and Morph/Skin composition on supported adapters.
   - [ ] **Imported GPU evaluation**: Preserve generated normal/tangent policies, authored Morph defaults, Morph-before-Skin ordering, and source geometry identity when routing imported primitives to GPU draws.
     - [x] **glTF GPU adapter**: Retained per-primitive sources, shared CPU/GPU weight resolution, instance-mapped palettes, and ordered Morph/direction/Skin composition.
     - [x] **Imported evaluation admission**: Weight-dependent aggregate GPU payload reports, per-call budgets before primitive dispatch, zero-weight source reuse accounting, and viewer evaluation limits separate from render preparation.
     - [x] **Imported source admission**: CPU-only aggregate retained-buffer planning, core payload checks before the first upload, per-occurrence source accounting, bind-direction snapshots, and viewer source budgets independent of evaluation.
     - [ ] **Source admission GPU validation**: Verify constructor budget boundaries, retained reports, direction-stage allocations and multi-primitive source lifetime on supported adapters.
     - [ ] **Evaluation budget GPU validation**: Verify signed/default/zero-weight plans, exact aggregate limits, retained output sizes, source reuse, and rejection before dispatch on supported adapters.
     - [x] **Imported GPU viewer**: Explicit CPU/GPU modes, retained material-coordinate inputs independent of image readiness, complete pose/bounds publication, and submitted-frame primitive selection in the existing model viewer.
     - [ ] **Imported viewer GPU validation**: Verify animated GLB assets, CPU/GPU pose parity, asynchronous selection, image loading, resize, authored cameras, and mode/reload changes on supported adapters.
     - [ ] **Generated tangents**: Preserve imported MikkTSpace policies on deformed GPU geometry, including vertex correspondence and handedness.
       - [x] **GPU triangle derivatives**: Retained indices and selected UVs, immutable input/face pairing, derivative directions and magnitudes, mirrored-orientation and degeneracy classification, and bounded payload admission.
       - [x] **Corner source preparation**: Bounded unshared expansion, bit-preserved vertex attributes and tangent-set identity, output-to-source correspondence, and shared glTF Morph topology preparation.
       - [x] **Deformation source remapping**: Output-to-source Morph/Skin rebinding, exact delta and normalized-weight preservation, shared joint bindings, identity-map sharing, and validation before allocation.
       - [ ] **GPU tangent groups**: Deformed position/normal/UV welding, connected orientation groups, corner weighting, and degenerate-face inheritance with fixed vertex correspondence.
         - [x] **Dynamic corner welding**: Exact deformed keys, normalized normals, selected UVs, deterministic earliest-corner representatives, bounded ping-pong sorting, and retained derivative/input pairing.
         - [x] **GPU edge adjacency**: Opposite-edge rank pairing, deterministic non-manifold relationships, distinct coincident/collinear policies, inherited-frame eligibility, and regular-frame orientation compatibility.
         - [x] **Regular corner groups**: Orientation-compatible connected components, deterministic minimum-corner representatives, bounded pointer doubling, and retained adjacency/input snapshots.
         - [ ] **Group GPU validation**: Verify long chains and cycles across workgroups, seams, mirror boundaries, point-only contact, failed/collapsed faces, and changing deformation snapshots on supported adapters.
         - [ ] **Inherited frames and weights**: Assign degenerate-frame directions and accumulate corner-weighted tangent contributions with fixed vertex correspondence.
           - [x] **Regular corner frames**: Normal-projected angle weights, sorted group-local accumulation, opposing-direction subgroups, retained corner correspondence, and explicit arithmetic/frame status.
           - [x] **Collapsed-face inheritance**: Matching welded keys, deterministic earliest noncollapsed donors including undefined frames, retained frame provenance, attribute-seam isolation, and unresolved/failed corner preservation without recursive donation.
           - [x] **Undefined-frame groups**: Ordered regular-seed priority, face-wide inherited orientation, connected undefined-corner assignment, zero-contribution averaging, and parallel detection with a regular-only fast path.
           - [ ] **Corner frame GPU validation**: Verify nonuniform angles, mirrored/opposing directions, deformed normals and positions, collapsed faces, donor selection across workgroups, seams, numeric failures, and retained snapshots against CPU tangent generation on supported adapters.
         - [ ] **Adjacency GPU validation**: Verify mirrored boundaries, non-manifold ranks, point-only contact, collapsed/collinear and failed faces, dynamic pairing, and retained results on supported adapters.
         - [ ] **Welding GPU validation**: Verify dynamic split/merge, shared indices, seams, signed zero, failed corners, multi-workgroup sorting, and retained frames on supported adapters.
       - [ ] **GPU tangent publication**: Normal-orthogonal frames, imported repair policy, handedness checks, Morph-to-Skin composition, and CPU MikkTSpace parity on supported adapters.
         - [x] **Fixed-order tangent vertices**: Initial tangent-source metadata, normal projection, explicit Strict/Inherit/Repair modes, triangle handedness rejection, independent repair tags, and canonical deformation output for Skin and render packing.
         - [x] **Combined tangent generation**: Reusable stage ownership, direct deformation-to-tangent evaluation, aggregate payload admission, retained output identity, and explicit normal-reconstruction ordering.
         - [x] **Imported tangent dispatch**: Selected-UV Repair generation after Morph and normal reconstruction, retained zero-weight base directions, ordered corner admission, Morph-to-Skin composition, and output-specific render source identity.
         - [ ] **Publication GPU validation**: Verify deformed frame projection, repair selection, mixed signs, original failure propagation, retained outputs, and Morph/Skin/render composition on supported adapters.
         - [ ] **CPU/GPU generation parity**: Verify welding keys, edge ordering, normalization, and degenerate-frame inheritance across core and imported evaluation paths, including generated tangents and zero-weight transitions.
           - [x] **Regular-frame thresholds**: Strict lower bounds for the UV determinant and derivative magnitudes, independent zero-UV classification, and undefined-frame inheritance eligibility.
           - [x] **Publication numeric admission**: Retained CPU UV-degeneracy classification, edge/derivative range checks, and unrepaired numeric failure status in every generation mode.
           - [x] **Projected normalization**: Unscaled f32 direction normalization, explicit underflow-to-undefined handling, subgroup-local repair eligibility, and independent bitangent validity.
           - [x] **Derivative normalization**: Unscaled f32 lengths, reciprocal direction normalization, length-before-determinant magnitudes, and early numeric-range failure propagation.
           - [x] **Normal-key precision**: Device-admitted f64 square sums and normalization, exact f32 input decoding, and ties-to-even key encoding with signed-zero and subnormal preservation.
           - [x] **CPU topology rules**: Exact-bit corner welding, deterministic representatives, and face-ordered opposite-edge pairing, including non-manifold edges.
           - [x] **Publication precision**: Device-admitted f64 normal projection and relative degeneracy threshold, wide geometric-area classification, and CPU-ordered derivative repair rounding.
           - [x] **Derivative area classification**: Shared f64 geometric-area rules across derivative records and tangent publication, independent of f32 direction eligibility and UV orientation.
     - [ ] **Imported GPU validation**: Verify signed/default weights, multiple primitives and instances, direction reconstruction, source identity, and retained outputs against CPU evaluation on supported adapters.
   - [x] **Nonblocking mesh readback**: Owned staging requests, pre-allocation byte limits, explicit completion polling, and independent CPU mesh publication.
   - [x] **Bounds reduction**: Reusable GPU position reduction, fixed-size nonblocking readback, per-request admission, invalid-output rejection, and explicit geometry/bounds pairing.
   - [ ] **Render integration**: Consume deformation buffers without CPU readback, with explicit bounds and query synchronization policies.
     - [x] **GPU vertex packing**: Shared material-coordinate and index inputs, retained render-format vertices, and indirect draw suppression for invalid deformation results.
     - [x] **Packed geometry status**: Fixed-size nonblocking validation readback, combined issue flags, deterministic first vertex/triangle locations, and request ownership independent of rendered frames.
     - [x] **Headless draw routing**: Bind packed outputs to objects across render channels, with conservative render bounds and explicit CPU query materialization.
     - [x] **Viewport draw routing**: Window-device sharing, frame-local packed resources, conservative bounds, cache invalidation, and explicit CPU interaction limits.
     - [ ] **GPU viewport interaction**: Associate displayed deformation frames with ID/depth queries, pointer coordinates, and asynchronous selection results without using original CPU mesh hits.
       - [x] **Submitted data capture**: Opt-in WGPU ID/depth passes paired with the color submission, source-frame identity, clipped raster coordinates, target admission, and retained regional readback.
       - [x] **Viewport event integration**: Retained capture handles, frame/camera/object/layout pairing, logical pointer queries, submission freshness checks, and latest-click scheduling in the scene example.
       - [ ] **Capture GPU validation**: Verify submission gating, GPU-deformed coverage, resize/clipping, cached color replay, retained outputs, nested UI captures, and device loss on supported adapters.
     - [x] **Interactive deformation comparison**: Shared CPU/GPU timeline, Morph/Skin controls, retained sources, paired GPU bounds, bounded pending work, and explicit backend failures in the scene example.
     - [ ] **Viewport GPU validation**: Verify multiple viewports, retained outputs, replacement, shadows, nested captures, and device recovery on supported adapters.

Independent extensions:

- [ ] **Skeleton editing**: Joint visualization, selectable helper geometry, transform handles, weight editing, and application-owned edit history.
- [ ] **Retargeting and rig controls**: Source/destination joint mappings, reference-pose alignment, chain controls, and character-specific semantics above core pose APIs.
- [ ] **Physics integration**: Stable body/collider-to-node bindings, rigid-body/joint offsets, fixed-step simulation, render interpolation, and animation/physics pose ownership. Use an external solver for contacts, friction, inertia, continuous collision detection, and physical joint constraints.
- [ ] **Simulation state**: Caller-owned reset, checkpoint/replay, cancellation, and publication of coherent pose/mesh snapshots. Absolute-time animation sampling does not reconstruct simulation history.
- [ ] **Deformable simulation**: Cloth/soft-body outputs through mesh updates, with explicit topology, normal, bounds, and query-update policies.

Collision geometry and participation are independent of render visibility and material
alpha. Render-mesh ray and AABB queries do not constitute a physics collision solver.
Units, rigid-transform conversion, and treatment of scale/shear belong to the integration
contract. Core APIs do not own a physics world, character controller, or editor workflow.

## Near-Term Core Order

1. **Material extension contract**: Shared surface coverage, application shading, and backend-owned render state.
2. **Material resources and standard inputs**: Retained parameter/texture bindings, lighting access, and pipeline variants.
3. **External deformation and attribute streams**: Reusable geometry processing with independent attribute updates.
4. **Additional mesh passes and frame consistency**: Outline-capable draws and coherent coverage/picking snapshots.

### Application Rendering Extensions

- [ ] **Extensible material shading**:
  - [ ] **Surface coverage contract**: One alpha/cutout evaluator for color, shadow, depth, object ID, and normal outputs; explicit allowed inputs and consistent deformed geometry. Keep view-dependent shading separate from coverage and define camera/light-view sampling behavior.
    - [x] **Backend surface separation**: Shared world-space vertex payloads for camera/shadow passes, one material surface evaluator, renderer-owned clipping, and RGB-only shading with centralized HDR clamping and alpha premultiplication.
  - [ ] **Shading contract**: Application WGSL functions with versioned inputs for world/view position, geometric and shading normals, tangents, UVs, vertex color, and camera data. Return linear HDR color; the backend owns output encoding and alpha premultiplication.
    - [x] **Backend material programs**: Bounded source assembly, typed surface/shading signatures, transitive helper permissions, explicit discard/global/entry-point rejection, and shared built-in compilation for viewport/headless pipelines.
    - [x] **Camera-space inputs**: Submitted world-to-view position/vector helpers restricted to shading, with perspective/orthographic viewing directions and example Toon/Sphere Map programs.
    - [x] **Standard surface factors**: Shading-only metallic, roughness, emission and occlusion sampling; standard maps and tangent-space normals available to custom primary and additional passes independently of the built-in lighting model.
  - [ ] **Lighting access**: Reusable direct-light direction, energy, attenuation, shadow visibility, and environment helpers without requiring the built-in PBR response.
    - [x] **Specular environment inputs**: Shading-only prefiltered radiance and split-sum BRDF helpers using renderer-owned textures, rotation, intensity, and roughness levels.
    - [ ] **Environment helper GPU validation**: Verify custom and built-in specular responses, direction and rotation, intensity, roughness levels, and inactive environments on supported adapters.
  - [ ] **Material resources**: Declared bounded parameter layouts, textures and samplers, immutable per-frame binding snapshots, independent data updates, and explicit format/color-space/UV requirements.
    - [x] **Program resource reflection**: Group 1 uniform/texture/sampler declarations, typed layouts, transitive coverage/shading usage, CPU admission, and enabled-device checks including standard bindings.
    - [x] **Bound resource snapshots**: Device-local shader/layout sources, checked bind-group creation, private uniform uploads, retained views/samplers with creation-device identity, full-snapshot payload budgets, and partial updates sharing unchanged resources.
    - [ ] **Resource snapshot GPU validation**: Verify shader/binding creation, uniform content across retained updates, texture view formats/dimensions, foreign devices, and failure/device-loss behavior on supported adapters.
  - [ ] **Pipeline integration**: Shared viewport/headless variants keyed by shader/layout and render state; resource-aware batching, admission, compilation diagnostics, cache reclamation, and device replacement. Invalid extensions fail explicitly.
    - [x] **Scene material attachment**: Retained snapshots on scene materials and draw packets, shared color/shadow/data variants, snapshot-aware batching, device checks, and explicit pipeline errors. Custom coverage uses GPU picking rather than CPU viewport routing.
    - [ ] **Pipeline GPU validation**: Verify custom clipping and retained parameter updates across all output channels, standard material factors and normal maps in primary/additional programs, built-in/custom pipeline switches, MSAA, shadows, GPU deformation, and device replacement.
  - [ ] **Coverage validation**: Compare masked coverage across outputs on supported adapters; define blended-surface ID/depth selection and shadow participation separately from color accumulation. Do not assume arbitrary shader coverage can be reproduced by CPU mesh queries.
- [ ] **External GPU deformation results**: Adopt application-produced attribute buffers through a checked result contract covering device ownership, vertex layout/count, base topology, submission ordering, status, and retained lifetime. Reuse bounds reduction, direction processing, and render packing without full vertex readback; define bounds invalidation and publication explicitly.
  - [x] **Canonical buffer adoption**: Exact 64-byte record admission, creation-device identity checks before backend access, zero-copy retention and independent GPU copies into `GpuDeformationOutput`; shared processing and explicit producer/lifetime contracts.
  - [x] **Packing input ownership**: Creation-device identity retained through generated and adopted deformation outputs, with owned-resource admission at the public packing entry before backend binding.
  - [ ] **External result GPU validation**: Verify queued producers, retained copies after source reuse, foreign devices, invalid records, direction processing, bounds, and packed rendering on supported adapters.
- [ ] **Dynamic attribute streams**: Independent UV-set, vertex-color, and declared custom-attribute updates with format/stride/count validation, stable topology, retained frame versions, and matching custom-shader inputs. Avoid rebuilding unchanged mesh attributes and indices.
  - [x] **Render-source UV/color snapshots**: Typed selected-set and linear-color replacements, bounded sparse uploads, retained GPU source versions, shared indices/pipelines, and unchanged base topology for deformation packing.
  - [x] **GPU UV/color inputs**: Device-owned packed buffer copies mixed with CPU streams, exact byte/usage admission, queue-ordered independent snapshots, and draw suppression for invalid GPU attribute values.
  - [ ] **Attribute snapshot GPU validation**: Verify repeated selections, mixed CPU/GPU updates, queued producers and source reuse, foreign devices, invalid GPU values, retained source/results, unchanged geometry, and alpha coverage across render and picking outputs on supported adapters.
  - [ ] **Custom attribute inputs**: Declared formats, independent GPU streams, bounded shader access, and consistent material-stage interpolation without repurposing standard UV/color channels.
    - [x] **Typed shader inputs**: Named packed 32-bit scalar/vector declarations, perspective/linear/flat interpolation, shared camera/shadow transport, protected vertex storage, and CPU/device-limit admission.
    - [x] **Stream binding and submission**: Private immutable stream uploads, exact packed-stride/count admission, independent updates, shared pipeline layouts, all-pass binding, and material-retained versions. Draw preparation rejects missing or mismatched vertex counts.
    - [x] **External attribute buffers**: Mixed CPU uploads, zero-copy retained buffers, and private GPU copies with creation-device checks, exact allocation/usage admission, partial replacement, and explicit producer and lifetime contracts.
    - [ ] **Custom stream GPU validation**: Verify retained/partial updates, queued external producers and source reuse, foreign devices, integer and smooth interpolation, masked coverage across channels/shadows, GPU-deformed geometry, viewport captures, and device replacement on supported adapters.
- [ ] **Additional mesh passes**: Reuse geometry with explicit culling, depth comparison/write/bias, blending, vertex offsets, and width attributes. Define pass ordering, expanded bounds, shadow participation, and object-ID ownership independently of color-only outlines.
  - [x] **Color pass submission**: Retained per-material pass lists, independent face/alpha/depth/blend controls, explicit stages around transparent surfaces, shared CPU/GPU geometry, bounded pipeline variants, and primary-owned shadow/data coverage.
  - [x] **Normal expansion**: Signed world-unit or raster-pixel displacement, bounded Float32 width streams, pass-specific vertex specialization, conservative expanded camera visibility, and unchanged primary data/shadow bounds.
  - [ ] **Mesh pass GPU validation**: Verify stage order, mirrored face visibility, depth bias/write/compare, blending, independent material and custom-stream snapshots, deformed geometry, viewport captures, and unchanged primary data/shadow outputs on supported adapters.
- [ ] **Coherent evaluated frames**: Compose final transforms, geometry/attribute results, material snapshots, and bounds into a retained submission with transactional validation and no application animation or physics policy in the renderer.
  - [x] **Object batches**: Retained scene updates combine final world transforms, CPU/GPU geometry with bounds, and material/attribute snapshots; validate target IDs and resource metadata without publishing partial results.
  - [x] **Geometry preparation**: Coupled packed-geometry validation and bounds readback, bounded per-request payload, retained ready pairs, terminal failures, and deferred geometry publication in the scene example.
  - [x] **Bounded batch preparation**: Aggregate working-payload admission and source/device preflight before packing, ordered all-or-nothing results, terminal failure cleanup, and viewer-wide primitive-batch limits.
  - [ ] **Batch preparation GPU validation**: Verify aggregate budget boundaries, source mismatches, result ordering, invalid late entries, cancellation, and retained geometry across multi-primitive viewer frames on supported adapters.
  - [x] **Imported batch preparation**: The model viewer publishes all primitive geometry/status/bounds results with retained poses and material coordinate selections, keeping the previous display batch on preparation failure.
  - [x] **Imported source rebinding**: Mesh-allocation-aware render-source reuse across direction regeneration and zero-weight transitions, with retained displayed batches.
  - [x] **Viewer packing cache**: One current source per node, aggregate source-plus-result admission before upload, coordinate/device preflight, and transactional replacement after batch request creation.
  - [ ] **Packing cache GPU validation**: Verify UV and mesh replacements, shared index retention, removed entries, and displayed output lifetime on supported adapters.
  - [ ] **Submission acceptance**: Verify combined GPU geometry, custom streams, material/pass changes, retained outputs and asynchronous coverage identities across viewport and headless rendering.
- [ ] **Deformed custom-material picking**: Bind regional ID/depth and coverage results to the submitted geometry, material resources, camera, and viewport identity; reject stale results and preserve retained-frame reads after updates.
  - [x] **Output provenance**: Opaque output identities carried through regional/full readbacks, picks, CPU coverage/labels and GPU labels, with mounted-viewport freshness checks before or after completion.
  - [x] **Regional metadata**: Headless and viewport region reads retain output extent, camera projection, identities and source rectangle for world reconstruction, depth comparison, coverage and labels.
  - [ ] **Picking acceptance**: Verify output identity propagation and stale-result rejection with combined GPU deformation, custom material clipping, retained frames, viewport layout changes and device replacement.

Toon responses, sphere-map coordinate generation, outline styles, and character-specific
material conventions belong to application or extension code built on these interfaces.

Keep multiple UI textures, capture-alpha picking, and focus/overlay support as independent GUI extensions.

Address performance, resource lifecycle, and platform validation alongside each feature.
