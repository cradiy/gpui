# Grouped edit occlusion

With the `wgpu` feature, `Scene::edit_occlusion` requests independent depth and
identity outputs for application-defined groups. Primary linear depth, normals
and object IDs continue to describe the final scene. Optional
[edit overlays](edit_overlays.md) add points and lines to color output without
altering those primary data channels.

Each group's output contains:

- Final-surface objects outside the group, using their existing geometry,
  transforms and material coverage rules.
- Caller-supplied auxiliary meshes for that group's self-occlusion.

Final-surface members of the group are omitted from its auxiliary output. This
preserves foreign occluders even where the primary output's nearest surface
belongs to the edited group. Other groups' auxiliary meshes are not included.

## Define a group

```rust,ignore
use gpui_3d::{AffineTransform, EditOcclusionGroup};

let scene = scene.edit_occlusion([
    EditOcclusionGroup::new(42, ["body".into(), "trim".into()])
        .mesh(control_mesh, AffineTransform::IDENTITY),
]);
```

Group IDs are stable application-defined `u64` values. Members use `Object::id`
identities and must each resolve to exactly one scene object. Groups must have
unique IDs and disjoint, nonempty membership. Duplicate or missing members fail
preparation. An empty auxiliary mesh set intentionally removes self-occlusion
while retaining all foreign scene objects.

Auxiliary meshes are opaque, double-sided CPU geometry with an explicit affine
world transform. Their coordinates, vertex order and topology are not modified.
Subdivision, model evaluation, selection state and editing semantics remain
application-owned. Final-surface objects may retain GPU deformation and custom
materials supported by the ordinary depth/ID renderer.

## Headless output

```rust,ignore
let frame = renderer.render(&scene, output_config)?;
let group = frame.gpu().occlusion_groups().iter()
    .find(|group| group.group_id() == 42)
    .expect("requested group");
assert_eq!(group.parent_frame_id(), frame.frame_id());
let depth = group.gpu().linear_depth().expect("group depth");
let identities = group.gpu().object_ids().expect("group identities");
let pending = group.gpu().readback()?;
```

Groups always produce R32Float linear depth and R32Uint identities, regardless
of the primary channel selection. Zero identity means background. Use the
group's `depth_background()` through `gpu()` to interpret empty depth samples.
Primary objects retain their primary output IDs; use `RenderedFrame::object`
to resolve them. `auxiliary_index(id)` resolves an auxiliary mesh's insertion
index. Auxiliary IDs are group-local and must not be interpreted as primary
object IDs or editable point/edge IDs.

## Viewport capture

```rust,ignore
let capture = ViewportPickCapture::new(64 * 1024 * 1024);
let element = viewport3d("editor", scene).pick_capture(capture.clone());
// After submission, obtain the matching viewport frame.
if let Some(frame) = capture.frame()? {
    for group in frame.occlusion_groups() {
        assert_eq!(group.parent_frame_id(), frame.frame_id());
    }
}
```

Attach the same capture on each render to access submitted outputs. Groups with
points or lines render in ordinary viewports without requiring a capture;
depth-only groups require a capture in viewports. Viewport and headless
group outputs share the same rendering implementation. Captured groups use the
same camera, clipped projection rectangle, raster dimensions and resolution
scale as their primary ID/depth output. `projection_rect()` reports that raster
rectangle. Use the primary capture's pixel mapping for all its groups.

## Lifetime, budgets and scope

Group output handles retain their textures across later draws, resizes and
renderer destruction. `parent_frame_id()` associates them with one submitted
primary output; identical scene contents in another submission have a different
identity. GPU consumers require the same device and queue ordering. Publication
means submission, not GPU completion. Readbacks use the existing bounded,
nonblocking readback API and share its admission limits.

Target budgets and `target_memory()` include all group output textures and
their depth attachments. Direct renderers provide
`validate_occlusion_target_memory` for preflight accounting, with the number of
occlusion groups and the number of groups containing points or lines. Primary
`geometry_memory()` describes primary geometry; each group reports its own.
The direct geometry limit conservatively includes group geometry and point/line
instance payloads. Element outputs report their own vertex payload and data draw
statistics. Primary draw statistics include group passes. Each group adds separate depth/ID draws;
only request groups currently needed for editing.

`group.gpu()` contains occlusion-source geometry IDs. `group.elements()` contains
separate caller-assigned point/line IDs and linear depth when the group has
elements. Neither replaces the primary object's identity output. Non-WGPU
backends do not produce these auxiliary outputs or render edit overlays.
