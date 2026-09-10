# Geometry

[3D viewports](viewport.md)

## Coordinates and geometry

World coordinates are right-handed, with positive Y up. The default camera is
at `[0, 0, 6]`, looking toward the origin. Angles are radians. `Camera::orbit`
orbits the origin; `Camera` exposes `eye`, `target`, `up`, `projection`,
`lens_shift`, and clip distances for explicit positioning. Projection matrices are column-major, with
camera forward along local -Z and hardware depth from zero to one. Scene units
are application-defined; camera distances and geometry must use the same units.

`Mesh::plane()` is a unit XY square facing positive Z. `Mesh::cube()` is a unit
cube centered at the origin. Both reuse shared geometry. `Mesh::new` accepts
vertices with position, normal and UV, plus counterclockwise triangle indices.
UV `(0, 0)` is at the top left. Faces render from both sides.

`Mesh::try_new(vertices, indices)` returns `Result<Mesh, MeshError>`. Vertices and
indices must be nonempty, index counts must be multiples of three, and each index
must reference an existing vertex. Position, normal and UV components must be
finite, including those of unused vertices. Errors identify an invalid index's
buffer offset or a non-finite vertex's attribute and component, all zero-based.
`Mesh::new` uses the same validation and panics on invalid input.

```rust
use gpui_3d::{Mesh, Vertex};

# fn main() -> Result<(), gpui_3d::MeshError> {
let vertices = [[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]]
    .map(|position| Vertex { position, normal: [0., 0., 1.], uv: [0.; 2] })
    .to_vec();
let mesh = Mesh::try_new(vertices, vec![0, 1, 2])?;
let triangle = &mesh.indices()[..3];
let first_position = mesh.vertices()[triangle[0] as usize].position;
# Ok(())
# }
```

`vertices()` and `indices()` borrow immutable storage; mesh clones share that
storage. `vertex_count()`, `index_count()` and `triangle_count()` report the stored
data, without filtering. `bounds()` computes mesh-local bounds of all vertices,
including unused ones, and permits zero extent.

Construction preserves ordering, repeated indices, zero normals and finite UVs
outside `[0, 1]`. It does not generate normals or remove degenerate triangles.
Zero-area triangles have no filled surface and are skipped by ray picking;
retaining their indices preserves subsequent `Hit::triangle_index` values.
Nonzero normals are normalized during shading. Geometry validation does not
guarantee that every camera or transform will yield numerically representable
rendering or intersection results.

Object transforms apply scale, X/Y/Z Euler rotation, then translation. Normals
use inverse-transpose transforms for nonuniform scale. Scale components must be
finite and nonzero. Camera clip distances must satisfy `0 < near < far`.
Perspective cameras allow positive infinity for `far`; orthographic cameras
require a finite far distance.

## Primitive meshes

`Mesh::sphere`, `Mesh::cylinder`, `Mesh::cone`, and `Mesh::subdivided_plane`
accept configuration structs and return `Result<Mesh, PrimitiveError>`.
Dimensions are finite positive scene units. Generation is synchronous; build a
mesh during resource preparation and clone it across objects to share storage.

```rust
use gpui_3d::{ConeOptions, CylinderOptions, Mesh, PlaneOptions, SphereOptions};

let sphere = Mesh::sphere(SphereOptions {
    radius: 0.7,
    segments: [64, 32],
})?;
let cylinder = Mesh::cylinder(CylinderOptions {
    height: 2.,
    segments: [48, 4],
    ..Default::default()
})?;
let cone = Mesh::cone(ConeOptions { capped: false, ..Default::default() })?;
let grid = Mesh::subdivided_plane(PlaneOptions {
    size: [4., 3.],
    segments: [16, 12],
})?;
# Ok::<(), gpui_3d::PrimitiveError>(())
```

Planes face +Z and default to a unit square with one interval per axis. Their
segments are `[horizontal, vertical]`, each at least one. Spheres are centered at
the origin with default radius 0.5 and `[longitude, latitude]` segments `[32, 16]`;
longitude requires at least three sectors and latitude at least two intervals.

Cylinders and cones use the Y axis, default radius 0.5, height 1, and
`[radial, height]` segments `[32, 1]`. Radial sectors require at least three and
height intervals at least one. `capped` defaults to true: both cylinder ends or
the cone base are closed. A cone's tip is at `+height/2` and its base at
`-height/2`.

All primitives provide outward counterclockwise triangles, unit normals, and
analytic tangent frames. Round side surfaces use U from +X toward +Z and V from
top to bottom. UV seams have coincident positions but distinct U values. Sphere
poles and cone tips use a separate vertex per sector with midpoint U; the sphere
normal remains axial at each pole. Flat cap normals and disk UVs use separate
vertices from the sides. Plane UV `(0, 0)` is upper-left. Bounds describe the
generated vertices, so coarse round meshes need not reach every ideal radial
extremum.

Generation rejects invalid dimensions or segment counts before allocation.
Each mesh is limited to 1,048,576 vertices and 6,291,456 indices. Dimensions that
collapse a generated triangle in f32 coordinates return `PrimitiveError::Degenerate`
with its triangle index. Primitive generation does not emit zero-area pole or
tip triangles.

## Normal generation

`Mesh::generate_normals(mode)` returns a `GeneratedNormals` mesh and an
output-to-source vertex map. Normals come from indexed positions and
counterclockwise winding, independently of existing normal values or UVs.

- `NormalMode::Flat` assigns each face's unit normal, splitting shared source
  vertices where the generated face normals differ. Coplanar faces can reuse
  vertices.
- `NormalMode::Smooth` sums face cross products at each shared source index and
  normalizes the sum, giving area-weighted smoothing. It does not weld coincident
  positions or smooth across separate source indices. There is no implicit
  crease-angle threshold.

```rust
use gpui_3d::{Mesh, NormalMode, Vertex};

let vertices = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
    .map(|position| Vertex { position, normal: [0.; 3], uv: [0.; 2] });
let source = Mesh::try_new(vertices.to_vec(), vec![0, 1, 2])?;
let generated = source.generate_normals(NormalMode::Flat)?;
let mesh = generated.mesh();
let source_index = generated.source_vertices()[0];
# let _ = (mesh, source_index);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Triangle order, winding, positions, and UVs remain unchanged. Output vertices
follow first use, omit unreferenced vertices, and do not merge distinct source
indices. Output bounds exclude omitted positions. Existing normals are replaced
and tangents are removed. Generate tangents from the resulting normals and UVs
when required by the material.

`NormalGenerationError` identifies zero-area triangles and undefined smooth
normals caused by cancelling incident faces. Generation returns an error instead
of removing triangles or choosing an arbitrary normal. Cross products and smooth
sums use f64; output normals are normalized f32 vectors. The source mesh remains
unchanged on success and failure.

`source_vertices()[output_index]` maps generated vertices to source attributes.
Use it to remap skin influences, position deltas, and external data. If tangent
generation also splits vertices, compose its map with the normal-generation map.
Vertex correspondence does not convert normal or tangent deltas to a new basis;
recompute those deltas when replacing their authored basis. To derive normals
from deformed positions, generate after evaluating the position deformation.
Generation is explicit CPU work, not an automatic rendering step.

## Fixed-topology updates

`Mesh::with_vertices(vertices, tangents)` returns an immutable snapshot with new
positions, normals, and UVs. Vertex count and triangle indices remain unchanged,
including unused vertices and degenerate triangles. Snapshots share index storage
but have independent vertex data, bounds, and lazy query indices. Earlier meshes,
evaluated scenes, and submitted headless outputs remain valid.

```rust
use gpui_3d::{Material, Mesh, Node, SceneGraph};

let source = Mesh::plane();
let mut vertices = source.vertices().to_vec();
for vertex in &mut vertices {
    vertex.position[2] += 0.25;
}
let updated = source.with_vertices(vertices, source.tangents().map(<[_]>::to_vec))?;
let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new().mesh(source, Material::color(gpui::rgb(0xffffff))))?;
graph.set_mesh(node, updated)?;
let evaluated = graph.evaluate()?;
# let _ = evaluated;
# Ok::<(), Box<dyn std::error::Error>>(())
```

All replacement attributes must be finite. Supply tangents appropriate for the
new normals; `None` explicitly omits them. Tangent validation orthogonalizes XYZ
and retains triangle handedness requirements. `MeshUpdateError` distinguishes
vertex count, geometry, and tangent failures. Replacements do not generate normals
or tangents automatically.

`SceneGraph::set_mesh` updates an existing mesh node and its local bounds without
changing its material, identity, hierarchy, or transform. Groups return
`SceneError::NoMesh`; failed updates leave the graph unchanged. Evaluate the graph
again for current world bounds and picking.

The WGPU renderer reuses vertex and index buffers from retired snapshots with
shared topology and equal vertex counts. Simultaneously visible snapshots keep
separate vertex contents. Replacement uploads are ordered before their draws,
including shadow and geometry-output passes. Upload staging storage is allocated
per replacement snapshot; cached replacements replay that copy when drawn.
Meshes kept in their original allocations require no vertex uploads.
Cache entries absent from the prepared scenes are released. Vertex/index count
changes require a new `Mesh`; batched instance streams are separate from vertex
replacement.

## Related topics

[Materials and tangent frames](materials.md#normal-maps-and-tangents).
