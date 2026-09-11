# Object submissions

[3D viewports](../viewport.md) · [Scene hierarchy](scenes.md)

`Scene::with_object_updates()` returns a retained scene containing final object
values. One batch can replace world transforms, CPU or GPU geometry, and complete
materials. The source scene remains unchanged on success and failure.

```rust
use gpui::rgb;
use gpui_3d::{AffineTransform, Material, Mesh, ObjectUpdate};

let submitted = scene.with_object_updates([
    (output_id, ObjectUpdate::new()
        .world(AffineTransform::from_translation([1., 0., 0.])?)
        .mesh(Mesh::cube())
        .material(Material::color(rgb(0x89c8ee)))),
])?;
```

Targets use output IDs from the source scene's `geometry_inputs()`. These IDs
are object indices plus one, not persistent identifiers across graph evaluations.
Duplicate, zero, and absent targets are rejected. Object order, application IDs,
node handles, camera, lights, and unmentioned properties are retained. World
transforms apply directly to individual objects; they do not propagate to other
objects or re-evaluate graph cameras, lights, or constraints.

CPU mesh replacement clears any previous GPU geometry and explicit render bounds.
UV sets and vertex colors are part of the replacement mesh. Complete material
replacement retains its custom program, parameter/texture snapshot, independent
custom vertex streams, and additional passes.

## GPU results

With native `wgpu` support, geometry and conservative local bounds are paired:

```rust
let submitted = scene.with_object_updates([
    (output_id, ObjectUpdate::new()
        .gpu_geometry(packed_geometry.clone(), local_bounds)
        .material(final_material)),
])?;
```

The geometry result supplies its matching base mesh. Packed UV and color updates
remain part of that result. CPU queries use base geometry at the submitted world
transform, not GPU-deformed vertices. Use rendered ID/depth queries for the GPU
pose. Bounds must conservatively enclose the GPU result; their numerical validity
does not prove that they enclose its vertices.

The batch checks device health and common device ownership, base meshes, active
material UV selections, required custom-stream counts, and pass width attributes,
including objects outside the camera. It performs no GPU commands or readbacks.
Externally shared buffers must remain immutable for as long as a retained scene
can use them. Submission does not copy these buffers or wait for external work.

## Validation and rendering

Scene parameters are checked before returning the result, without resolving
images. Mesh and material replacements are validated as a combination, not against
each other's previous values. No partial scene is published. Iterator side
effects and application-owned GPU work are outside this operation.

The returned `Scene` works with `viewport3d()` and `HeadlessRenderer::render()`;
no separate GPU override list is needed. Nonempty updates invalidate preparation
and spatial-query caches for the result, while old scenes and prepared frames
remain usable. An empty valid batch shares the original cache identity.

Output dimensions, the actual target device, allocation budgets, image readiness,
pipeline creation, and GPU completion remain renderer checks. This operation
does not certify those conditions or assign a rendered-frame identity. Retain
the viewport capture or headless output with its corresponding pick/readback.
