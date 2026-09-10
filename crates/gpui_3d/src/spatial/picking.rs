use crate::{
    ObjectId, Scene,
    math::{cross, dot, sub, transform, unit},
};
use gpui::{Bounds, Pixels, Point, RenderImage};
use std::sync::Arc;

/// How a surface participates in picking, independently of rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickBehavior {
    /// Return surface hits, including hits on unnamed objects.
    #[default]
    Target,
    /// Block surfaces behind this object without returning a hit.
    Occlude,
    /// Allow picking through this object, including its opaque pixels.
    Ignore,
}

/// Borrowed object identity returned by bounds queries or passed to query filters.
#[derive(Clone, Copy, Debug)]
pub struct QueryObject<'a> {
    /// Index in this scene's flattened list, not a persistent identity.
    pub object_index: usize,
    /// Graph identity, absent for objects constructed directly in a flat scene.
    pub node: Option<crate::NodeHandle>,
    /// Application identity; unnamed objects have no ID.
    pub object_id: Option<&'a ObjectId>,
    /// Authored picking behavior. Ray and screen queries apply it; bounds queries do not.
    pub pick_behavior: PickBehavior,
}

pub(crate) enum PickSurface {
    Absent,
    Solid,
    Image(Arc<RenderImage>),
}

pub(crate) struct PickSnapshot {
    pub scene: Scene,
    pub bounds: Bounds<Pixels>,
    pub surfaces: Vec<PickSurface>,
}

impl PickSnapshot {
    pub fn pick(&self, position: Point<Pixels>) -> Option<Hit> {
        self.scene.pick_query(
            self.bounds,
            position,
            |_| true,
            |index, _, triangle, barycentric| match &self.surfaces[index] {
                PickSurface::Absent => None,
                PickSurface::Solid => Some(1.),
                PickSurface::Image(image) => {
                    let object = &self.scene.objects[index];
                    let indices = &object.mesh.indices()[triangle * 3..][..3];
                    let mut uv = [0.; 2];
                    for (corner, &vertex) in indices.iter().enumerate() {
                        let coordinate =
                            object.mesh.uv_at(object.material.uv_set, vertex as usize)?;
                        for component in 0..2 {
                            uv[component] += coordinate[component] * barycentric[corner];
                        }
                    }
                    let size = image.size(0);
                    (size.width.0 > 0 && size.height.0 > 0 && image.as_bytes(0).is_some()).then(
                        || image_alpha(image, uv, self.scene.objects[index].material.sampling),
                    )
                }
            },
        )
    }
}

fn image_alpha(image: &RenderImage, uv: [f32; 2], sampling: crate::TextureSampling) -> f32 {
    let size = image.size(0);
    let Some(bytes) = image.as_bytes(0) else {
        return 0.;
    };
    if size.width.0 <= 0 || size.height.0 <= 0 || !uv.iter().all(|v| v.is_finite()) {
        return 0.;
    }
    let Some(uv) = sampling.transform.transform(uv) else {
        return 0.;
    };
    use crate::{TextureAddressMode as Address, TextureFilter};
    let address = |value: f32, mode| match mode {
        Address::Clamp => value.clamp(0., 1.),
        Address::Repeat => value - value.floor(),
        Address::Mirror => {
            let period = value - (value * 0.5).floor() * 2.;
            period.min(2. - period)
        }
    };
    let width = size.width.0;
    let height = size.height.0;
    let x = address(uv[0], sampling.address_u) * width as f32 - 0.5;
    let y = address(uv[1], sampling.address_v) * height as f32 - 0.5;
    let texel = |value: i32, extent: i32, mode| match mode {
        Address::Repeat => value.rem_euclid(extent),
        _ => value.clamp(0, extent - 1),
    };
    let alpha = |x, y| {
        let x = texel(x, width, sampling.address_u) as usize;
        let y = texel(y, height, sampling.address_v) as usize;
        f32::from(bytes[(y * width as usize + x) * 4 + 3]) / 255.
    };
    if sampling.magnification_filter() == TextureFilter::Nearest {
        return alpha((x + 0.5).floor() as i32, (y + 0.5).floor() as i32);
    }
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x1 = x0 + 1;
    let y1 = y0 + 1;
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    mix(
        mix(alpha(x0, y0), alpha(x1, y0), x - x.floor()),
        mix(alpha(x0, y1), alpha(x1, y1), x - x.floor()),
        y - y.floor(),
    )
}

/// The nearest eligible triangle intersection. Screen picking applies the
/// camera's clip range; world-ray queries do not.
#[derive(Clone, Debug)]
pub struct Hit {
    /// Graph identity, or `None` for an object built directly in a flat scene.
    pub node: Option<crate::NodeHandle>,
    /// Application identity, or `None` for an unnamed object.
    pub object_id: Option<ObjectId>,
    /// Index in this scene's flattened object list; not a stable identity.
    pub object_index: usize,
    /// Triangle index in the mesh's index buffer.
    pub triangle_index: usize,
    /// Intersection position in world space.
    pub position: [f32; 3],
    /// Interpolated world-space shading normal, flipped on back faces.
    pub normal: [f32; 3],
    /// Interpolated set-zero coordinates, independent of material sampling.
    /// `(0, 0)` is the top left.
    pub uv: [f32; 2],
    /// Weights corresponding to the triangle's three indexed vertices.
    pub barycentric: [f32; 3],
    /// World-space distance from the ray origin. Orthographic camera rays start
    /// on the eye plane, so this is forward depth rather than distance to the eye.
    pub distance: f32,
}

impl Scene {
    pub(crate) fn query_object(&self, object_index: usize) -> QueryObject<'_> {
        let object = &self.objects[object_index];
        QueryObject {
            object_index,
            node: object.node,
            object_id: object.id.as_ref(),
            pick_behavior: object.pick_behavior,
        }
    }

    /// Finds the nearest geometric surface at a logical window position.
    ///
    /// Material face visibility is respected. Unnamed objects still occlude other objects.
    /// Material and interpolated vertex alpha are respected; texture alpha, image availability,
    /// ancestor clipping, and effect deformation are not sampled by this query.
    /// Viewport callbacks also sample prepared image alpha and use GPUI hitbox routing.
    pub fn pick(&self, bounds: Bounds<Pixels>, position: Point<Pixels>) -> Option<Hit> {
        self.pick_where(bounds, position, |_| true)
    }

    /// Geometric world-ray query, independent of the scene camera and its clip range.
    /// Respects material face visibility, alpha and picking behavior, but does not resolve image alpha.
    pub fn raycast(&self, ray: crate::Ray) -> Option<Hit> {
        self.raycast_where(ray, |_| true)
    }

    /// Screen picking restricted to candidates accepted by `filter`.
    /// Rejected objects neither return hits nor occlude accepted objects. Accepted
    /// objects retain their authored picking behavior and material/vertex-alpha rules.
    /// Filtering does not alter rendering, scene identity, or cached spatial data.
    /// The predicate runs at most once per visited BVH candidate, in unspecified
    /// order; it is not an enumeration of every scene object. Texture alpha is not
    /// resolved. Viewport bounds and camera clipping match [`Scene::pick`].
    pub fn pick_where(
        &self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        filter: impl FnMut(QueryObject<'_>) -> bool,
    ) -> Option<Hit> {
        self.pick_query(bounds, position, filter, |_, _, _, _| Some(1.))
    }

    /// World-ray query restricted by object identity or application policy.
    /// Filtering has the same candidate and occlusion semantics as
    /// [`Scene::pick_where`], without viewport or camera clipping.
    pub fn raycast_where(
        &self,
        ray: crate::Ray,
        filter: impl FnMut(QueryObject<'_>) -> bool,
    ) -> Option<Hit> {
        self.trace(ray, |_| true, |_, _, _, _| Some(1.), filter)
    }

    fn pick_query(
        &self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        filter: impl FnMut(QueryObject<'_>) -> bool,
        alpha: impl Fn(usize, [f32; 2], usize, [f32; 3]) -> Option<f32>,
    ) -> Option<Hit> {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let x = f32::from(position.x - bounds.origin.x);
        let y = f32::from(position.y - bounds.origin.y);
        if ![width, height, x, y].iter().all(|v| v.is_finite())
            || width <= 0.
            || height <= 0.
            || x < 0.
            || x >= width
            || y < 0.
            || y >= height
        {
            return None;
        }
        let camera = self.camera;
        let [_, _, backward] = camera.axes().ok()?;
        let ray = camera.screen_to_ray(bounds, position).ok()?;
        self.trace(
            ray,
            |position| {
                let depth = -dot(sub(position, camera.eye), backward);
                depth >= camera.near && depth < camera.far
            },
            alpha,
            filter,
        )
    }

    fn trace(
        &self,
        ray: crate::Ray,
        within: impl Fn([f32; 3]) -> bool,
        alpha: impl Fn(usize, [f32; 2], usize, [f32; 3]) -> Option<f32>,
        mut filter: impl FnMut(QueryObject<'_>) -> bool,
    ) -> Option<Hit> {
        self.trace_with(
            ray,
            within,
            alpha,
            |visit| {
                self.visit_objects(ray, |object_index| {
                    if filter(self.query_object(object_index)) {
                        visit(object_index);
                    }
                });
            },
            |mesh, model, ray, visit| mesh.visit_triangles(model, ray, visit),
        )
    }

    fn trace_with(
        &self,
        ray: crate::Ray,
        within: impl Fn([f32; 3]) -> bool,
        alpha: impl Fn(usize, [f32; 2], usize, [f32; 3]) -> Option<f32>,
        objects: impl FnOnce(&mut dyn FnMut(usize)),
        mut candidates: impl FnMut(&crate::Mesh, crate::math::Matrix, crate::Ray, &mut dyn FnMut(usize)),
    ) -> Option<Hit> {
        let direction = ray.direction();
        let mut closest: Option<Hit> = None;
        let mut visit_object = |object_index: usize| {
            let object = &self.objects[object_index];
            if object.pick_behavior == PickBehavior::Ignore
                || !object.material.alpha_visible(object.material.color.a)
            {
                return;
            }
            let (model, normal_matrix) = object.matrices();
            let [a, b, c]: [[f64; 3]; 3] = std::array::from_fn(|column| {
                std::array::from_fn(|row| f64::from(model[column][row]))
            });
            let mirrored = a[0] * (b[1] * c[2] - b[2] * c[1])
                + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0])
                < 0.;
            let mut visit = |triangle_index: usize| {
                let indices = &object.mesh.indices()[triangle_index * 3..][..3];
                let vertices: [_; 3] =
                    std::array::from_fn(|i| &object.mesh.0.vertices()[indices[i] as usize]);
                let world = vertices.map(|v| {
                    let p = transform(model, [v.position[0], v.position[1], v.position[2], 1.]);
                    [p[0], p[1], p[2]]
                });
                let Some((distance, barycentric, front)) =
                    intersect(ray.origin(), direction, world, true)
                else {
                    return;
                };
                let front = front != mirrored;
                if !object.material.double_sided && !front {
                    return;
                }
                if closest.as_ref().is_some_and(|hit| {
                    distance > hit.distance
                        || (distance == hit.distance
                            && (object_index, triangle_index)
                                >= (hit.object_index, hit.triangle_index))
                }) {
                    return;
                }
                let position = ray.at(distance);
                if !within(position) {
                    return;
                }
                let uv = std::array::from_fn(|i| {
                    (0..3).map(|j| vertices[j].uv[i] * barycentric[j]).sum()
                });
                let Some(alpha) = alpha(object_index, uv, triangle_index, barycentric) else {
                    return;
                };
                let vertex_alpha = object.mesh.vertex_colors().map_or(1., |colors| {
                    (0..3)
                        .map(|corner| colors[indices[corner] as usize][3] * barycentric[corner])
                        .sum::<f32>()
                });
                if !object
                    .material
                    .alpha_visible(alpha * object.material.color.a * vertex_alpha)
                {
                    return;
                }
                let local_normal = std::array::from_fn::<_, 3, _>(|i| {
                    (0..3).map(|j| vertices[j].normal[i] * barycentric[j]).sum()
                });
                let n = transform(
                    normal_matrix,
                    [local_normal[0], local_normal[1], local_normal[2], 0.],
                );
                let normal = unit([n[0], n[1], n[2]]).map(|v| if front { v } else { -v });
                closest = Some(Hit {
                    node: object.node,
                    object_id: object.id.clone(),
                    object_index,
                    triangle_index,
                    position,
                    normal,
                    uv,
                    barycentric,
                    distance,
                });
            };
            candidates(&object.mesh, model, ray, &mut visit);
        };
        objects(&mut visit_object);
        closest.filter(|hit| self.objects[hit.object_index].pick_behavior != PickBehavior::Occlude)
    }
}

fn intersect(
    origin: [f32; 3],
    direction: [f32; 3],
    vertices: [[f32; 3]; 3],
    bounded: bool,
) -> Option<(f32, [f32; 3], bool)> {
    let edge1 = sub(vertices[1], vertices[0]);
    let edge2 = sub(vertices[2], vertices[0]);
    let p = cross(direction, edge2);
    let determinant = dot(edge1, p);
    let tolerance = 1e-7 * (dot(edge1, edge1) * dot(edge2, edge2)).sqrt();
    if !determinant.is_finite() || determinant.abs() <= tolerance {
        return None;
    }
    let offset = sub(origin, vertices[0]);
    let u = dot(offset, p) / determinant;
    let q = cross(offset, edge1);
    let v = dot(direction, q) / determinant;
    let distance = dot(edge2, q) / determinant;
    if (bounded && (!(0. ..=1.).contains(&u) || v < 0. || u + v > 1.))
        || !distance.is_finite()
        || distance < 0.
    {
        return None;
    }
    Some((distance, [1. - u - v, u, v], determinant > 0.))
}

pub(crate) struct DragProjection {
    camera: crate::Camera,
    bounds: Bounds<Pixels>,
    world: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
}

impl DragProjection {
    pub fn new(snapshot: &PickSnapshot, hit: &Hit) -> Self {
        let object = &snapshot.scene.objects[hit.object_index];
        let indices = &object.mesh.0.indices()[hit.triangle_index * 3..][..3];
        let vertices: [_; 3] =
            std::array::from_fn(|i| &object.mesh.0.vertices()[indices[i] as usize]);
        let (model, _) = object.matrices();
        Self {
            camera: snapshot.scene.camera,
            bounds: snapshot.bounds,
            world: vertices.map(|v| {
                let p = transform(model, [v.position[0], v.position[1], v.position[2], 1.]);
                [p[0], p[1], p[2]]
            }),
            uv: vertices.map(|v| v.uv),
        }
    }

    pub fn project(&self, position: Point<Pixels>) -> Option<[f32; 2]> {
        let ray = self.camera.screen_to_ray(self.bounds, position).ok()?;
        let (_, weights, _) = intersect(ray.origin(), ray.direction(), self.world, false)?;
        let uv = std::array::from_fn(|i| (0..3).map(|j| self.uv[j][i] * weights[j]).sum::<f32>());
        uv.iter().all(|v| v.is_finite()).then_some(uv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Material, Mesh, Object, Transform};
    use gpui::{point, px, rgb, size};

    fn bounds() -> Bounds<Pixels> {
        Bounds::new(point(px(70.), px(40.)), size(px(640.), px(360.)))
    }

    fn project(camera: Camera, bounds: Bounds<Pixels>, position: [f32; 3]) -> Point<Pixels> {
        let clip = transform(
            camera.matrix(bounds.size.width / bounds.size.height),
            [position[0], position[1], position[2], 1.],
        );
        bounds.origin
            + point(
                bounds.size.width * (clip[0] / clip[3] + 1.) * 0.5,
                bounds.size.height * (1. - clip[1] / clip[3]) * 0.5,
            )
    }

    fn plane() -> Object {
        Object::new(Mesh::plane(), Material::color(rgb(0xffffff)))
    }

    fn layered_grid(side: usize) -> Mesh {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for z in [0., -0.5] {
            for y in 0..side {
                for x in 0..side {
                    let first = vertices.len() as u32;
                    for [dx, dy] in [[0., 0.], [1., 0.], [1., 1.], [0., 1.]] {
                        let u = (x as f32 + dx) / side as f32;
                        let v = (y as f32 + dy) / side as f32;
                        vertices.push(crate::Vertex {
                            position: [u * 2. - 1., v * 2. - 1., z],
                            normal: [0., 0., 1.],
                            uv: [u, v],
                        });
                    }
                    indices.extend([0, 1, 2, 0, 2, 3].map(|i| first + i));
                }
            }
        }
        // Overlapping and degenerate primitives must retain their input IDs.
        indices.extend([0, 1, 2, 0, 0, 0]);
        Mesh::new(vertices, indices)
    }

    fn same_hit(actual: Option<Hit>, expected: Option<Hit>) {
        let (actual, expected) = match (actual, expected) {
            (None, None) => return,
            (Some(actual), Some(expected)) => (actual, expected),
            pair => panic!("different query results: {pair:?}"),
        };
        assert_eq!(actual.object_index, expected.object_index);
        assert_eq!(actual.object_id, expected.object_id);
        assert_eq!(actual.node, expected.node);
        assert_eq!(actual.triangle_index, expected.triangle_index);
        assert_eq!(actual.distance, expected.distance);
        assert_eq!(actual.position, expected.position);
        assert_eq!(actual.normal, expected.normal);
        assert_eq!(actual.uv, expected.uv);
        assert_eq!(actual.barycentric, expected.barycentric);
    }

    #[test]
    fn bvh_matches_exhaustive_queries_across_affine_instances_and_filters() {
        use crate::{AffineTransform, Node, Ray, SceneGraph};
        let mesh = layered_grid(16);
        let mut graph = SceneGraph::new();
        let group = graph.insert(None, Node::new()).unwrap();
        let node = graph
            .insert(
                Some(group),
                Node::new()
                    .id("grid")
                    .mesh(mesh.clone(), Material::color(rgb(0xffffff))),
            )
            .unwrap();
        graph
            .insert(
                Some(group),
                Node::new()
                    .id("overlap")
                    .mesh(mesh, Material::color(rgb(0xffffff))),
            )
            .unwrap();
        let poses = [
            AffineTransform::default(),
            AffineTransform::from_matrix([
                [-2., 0.3, 0.1, 0.],
                [0.5, 0.7, 0.2, 0.],
                [0.2, -0.1, 1.3, 0.],
                [3., -2., 1., 1.],
            ])
            .unwrap(),
            AffineTransform::from_trs(
                [10000., -20000., 30000.],
                [0.3, 0.5, -0.1, 0.8],
                [100., -200., 50.],
            )
            .unwrap(),
            AffineTransform::from_trs([0.; 3], [0.2, 0.3, 0.1, 0.9], [0.01, 0.02, 0.03]).unwrap(),
        ];
        let mut hits = 0;
        let mut previous = graph.evaluate().unwrap();
        previous.prepare_spatial_index();
        for pose in poses {
            graph.set_transform(group, pose).unwrap();
            let evaluated = graph.evaluate().unwrap();
            evaluated.prepare_spatial_index_from(&previous);
            let mut scene = evaluated.scene(Camera::default());
            for mode in [
                PickBehavior::Target,
                PickBehavior::Occlude,
                PickBehavior::Ignore,
            ] {
                scene.objects[0].pick_behavior = mode;
                for y in 0..9 {
                    for x in 0..9 {
                        for side in [-1., 1.] {
                            let x = x as f32 * 0.3 - 1.2;
                            let y = y as f32 * 0.3 - 1.2;
                            let origin = pose.transform_point([x, y, side * 2.]);
                            let target = pose.transform_point([x + 0.05, y - 0.03, -0.25]);
                            let ray = Ray::new(origin, sub(target, origin)).unwrap();
                            let within = |p: [f32; 3]| {
                                pose.inverse().transform_point(p)[2] >= -0.4 || x > 0.
                            };
                            let alpha = |index, uv: [f32; 2], _, _| {
                                Some(if index == 0 && uv[0] > 0.4 { 0. } else { 1. })
                            };
                            let expected = scene.trace_with(
                                ray,
                                within,
                                alpha,
                                |visit| (0..scene.objects.len()).for_each(visit),
                                |mesh, _, _, visit| {
                                    for triangle in 0..mesh.triangle_count() {
                                        visit(triangle);
                                    }
                                },
                            );
                            let actual = scene.trace(ray, within, alpha, |_| true);
                            hits += usize::from(actual.is_some());
                            same_hit(actual, expected);
                        }
                    }
                }
            }
            assert_eq!(evaluated.node(node).unwrap().world, pose);
            previous = evaluated;
        }
        assert!(hits > 100);
    }

    #[test]
    fn bvh_prunes_dense_meshes_and_reuses_the_index_after_motion() {
        use crate::{AffineTransform, Ray};
        let mesh = layered_grid(64);
        let clone = mesh.clone();
        assert!(mesh.1.get().is_none());
        std::thread::spawn(move || clone.prepare_spatial_index())
            .join()
            .unwrap();
        let index = mesh.1.get().unwrap();
        let rays = [
            Ray::new([0.213, -0.317, 2.], [0., 0., -1.]).unwrap(),
            Ray::new([4., 4., 2.], [0., 0., -1.]).unwrap(),
            Ray::new([0., 0., 2.], [1., 0., 0.]).unwrap(),
            Ray::new([1., 1., 0.], [0., 0., -1.]).unwrap(),
            Ray::new([0., 0., -0.25], [0., 0., -1.]).unwrap(),
        ];
        let scene = Scene::new().object(Object::new(mesh.clone(), Material::color(rgb(0xffffff))));
        for ray in rays {
            let mut tested = 0;
            let actual = scene.trace_with(
                ray,
                |_| true,
                |_, _, _, _| Some(1.),
                |visit| scene.visit_objects(ray, visit),
                |mesh, model, ray, visit| {
                    mesh.visit_triangles(model, ray, |triangle| {
                        tested += 1;
                        visit(triangle);
                    });
                },
            );
            let expected = scene.trace_with(
                ray,
                |_| true,
                |_, _, _, _| Some(1.),
                |visit| (0..scene.objects.len()).for_each(visit),
                |mesh, _, _, visit| {
                    for triangle in 0..mesh.triangle_count() {
                        visit(triangle);
                    }
                },
            );
            same_hit(actual, expected);
            assert!(
                tested < mesh.triangle_count() / 100,
                "tested {tested} triangles"
            );
        }
        let mut moved = scene.clone();
        moved.objects[0].transform.position = [10., 0., 0.];
        moved.spatial_index = Arc::default();
        assert!(moved.raycast(rays[0]).is_none());
        assert!(
            moved
                .raycast(Ray::new([10.213, -0.317, 2.], [0., 0., -1.]).unwrap())
                .is_some()
        );
        assert!(std::ptr::eq(index, moved.objects[0].mesh.1.get().unwrap()));
        assert!(scene.raycast(rays[0]).is_some());
        let mut tested = 0;
        mesh.visit_triangles(AffineTransform::default().matrix(), rays[1], |_| {
            tested += 1
        });
        assert_eq!(tested, 0);
    }

    fn alpha_image(width: u32, height: u32, alpha: &[u8]) -> Arc<RenderImage> {
        let bytes = alpha.iter().flat_map(|a| [255, 255, 255, *a]).collect();
        Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::from_raw(width, height, bytes).unwrap(),
        )]))
    }

    #[test]
    fn object_index_prunes_instances_and_survives_camera_changes_and_append() {
        use crate::{Projection, Ray};
        let plane = Mesh::plane();
        let mesh = Mesh::new(plane.vertices().to_vec(), plane.indices().to_vec());
        let mut scene = Scene::new();
        for y in 0..64 {
            for x in 0..64 {
                scene = scene.object(
                    Object::new(mesh.clone(), Material::color(rgb(0xffffff))).position([
                        x as f32 * 2.,
                        y as f32 * 2.,
                        0.,
                    ]),
                );
            }
        }
        scene.prepare_spatial_index();
        assert!(
            mesh.1.get().is_none(),
            "object preparation must not build triangle indices"
        );
        let ray = Ray::new([40.1, 30.2, 5.], [0., 0., -1.]).unwrap();
        let mut tested = 0;
        let actual = scene.trace_with(
            ray,
            |_| true,
            |_, _, _, _| Some(1.),
            |visit| {
                scene.visit_objects(ray, |index| {
                    tested += 1;
                    visit(index);
                });
            },
            |mesh, model, ray, visit| mesh.visit_triangles(model, ray, visit),
        );
        let expected = scene.trace_with(
            ray,
            |_| true,
            |_, _, _, _| Some(1.),
            |visit| (0..scene.objects.len()).for_each(visit),
            |mesh, _, _, visit| (0..mesh.triangle_count()).for_each(visit),
        );
        assert_eq!(actual.as_ref().unwrap().object_index, 15 * 64 + 20);
        same_hit(actual, expected);
        assert!(
            tested < scene.objects.len() / 100,
            "visited {tested} objects"
        );
        let mut missed = 0;
        scene.visit_objects(Ray::new([-5., -5., 5.], [0., 0., -1.]).unwrap(), |_| {
            missed += 1
        });
        assert_eq!(missed, 0);
        let camera = Camera {
            projection: Projection::Orthographic { vertical_size: 3. },
            eye: [40.1, 30.2, 5.],
            target: [40.1, 30.2, 0.],
            ..Camera::default()
        };
        let other_camera = scene.clone().camera(camera);
        assert!(Arc::ptr_eq(
            &scene.spatial_index,
            &other_camera.spatial_index
        ));
        same_hit(
            other_camera.pick(bounds(), bounds().center()),
            scene.raycast(ray),
        );
        let updated = scene.clone().object(
            Object::new(mesh, Material::color(rgb(0xffffff)))
                .id("front")
                .position([40., 30., 1.]),
        );
        assert!(!Arc::ptr_eq(&scene.spatial_index, &updated.spatial_index));
        assert_eq!(
            updated.raycast(ray).unwrap().object_id,
            Some("front".into())
        );
        assert_eq!(scene.raycast(ray).unwrap().object_index, 15 * 64 + 20);
        assert!(Scene::new().raycast(ray).is_none());
    }

    #[test]
    fn evaluated_object_indices_follow_hierarchy_edits_without_changing_old_snapshots() {
        use crate::{AffineTransform, Node, Ray, ReparentMode, SceneGraph};
        let mut graph = SceneGraph::new();
        let parent = graph.insert(None, Node::new()).unwrap();
        let surface = graph
            .insert(
                Some(parent),
                Node::new()
                    .id("surface")
                    .mesh(Mesh::plane(), Material::color(rgb(0xffffff))),
            )
            .unwrap();
        let ray = Ray::new([0.1, 0.2, 5.], [0., 0., -1.]).unwrap();
        let old = graph.evaluate().unwrap();
        old.prepare_spatial_index();
        let before = old.scene(Camera::default());
        let cloned = old.scene(Camera::orbit(0.3, 0.2, 7.));
        assert!(Arc::ptr_eq(&before.spatial_index, &cloned.spatial_index));
        assert_eq!(before.raycast(ray).unwrap().node, Some(surface));

        graph
            .set_transform(
                parent,
                AffineTransform::from_translation([4., 0., 0.]).unwrap(),
            )
            .unwrap();
        let translated = graph.evaluate().unwrap();
        let moved = translated.scene(Camera::default());
        let moved_ray = Ray::new([4.1, 0.2, 5.], [0., 0., -1.]).unwrap();
        assert!(!Arc::ptr_eq(&before.spatial_index, &moved.spatial_index));
        assert!(moved.raycast(ray).is_none());
        assert_eq!(moved.raycast(moved_ray).unwrap().node, Some(surface));
        graph.set_visible(parent, false).unwrap();
        assert!(
            graph
                .evaluate()
                .unwrap()
                .scene(Camera::default())
                .raycast(moved_ray)
                .is_none()
        );
        graph.set_visible(parent, true).unwrap();
        graph
            .reparent(surface, None, ReparentMode::KeepWorld)
            .unwrap();
        graph.remove_subtree(parent).unwrap();
        assert_eq!(
            graph
                .evaluate()
                .unwrap()
                .scene(Camera::default())
                .raycast(moved_ray)
                .unwrap()
                .node,
            Some(surface)
        );
        graph.remove_subtree(surface).unwrap();
        let replacement = graph
            .insert(
                None,
                Node::new().mesh(Mesh::plane(), Material::color(rgb(0xffffff))),
            )
            .unwrap();
        assert_ne!(replacement, surface);
        assert_eq!(
            graph
                .evaluate()
                .unwrap()
                .scene(Camera::default())
                .raycast(ray)
                .unwrap()
                .node,
            Some(replacement)
        );
        assert_eq!(before.raycast(ray).unwrap().node, Some(surface));
        assert_eq!(moved.raycast(moved_ray).unwrap().node, Some(surface));
        drop(graph);
        same_hit(before.raycast(ray), cloned.raycast(ray));
    }

    #[test]
    fn vertex_alpha_interpolates_and_combines_with_texture_and_material() {
        let mesh = Mesh::new(
            [[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]]
                .map(|position| crate::Vertex {
                    position,
                    normal: [0., 0., 1.],
                    uv: [0.; 2],
                })
                .to_vec(),
            vec![0, 1, 2],
        )
        .with_vertex_colors(vec![[1., 0., 0., 0.], [0., 1., 0., 1.], [0., 0., 1., 0.]])
        .unwrap();
        let mut snapshot = PickSnapshot {
            scene: Scene::new()
                .object(plane().id("rear").scale([4.; 3]))
                .object(
                    Object::new(mesh, Material::color(rgb(0xffffff)).alpha_cutoff(0.5))
                        .id("front")
                        .position([0., 0., 1.]),
                ),
            bounds: bounds(),
            surfaces: vec![PickSurface::Solid, PickSurface::Solid],
        };
        let at = |x| project(Camera::default(), bounds(), [x, -0.5, 1.]);
        for (x, expected) in [(-0.6, "rear"), (0.6, "front")] {
            assert_eq!(
                snapshot.pick(at(x)).unwrap().object_id,
                Some(expected.into())
            );
            let ray = crate::Ray::new([x, -0.5, 3.], [0., 0., -1.]).unwrap();
            assert_eq!(
                snapshot.scene.raycast(ray).unwrap().object_id,
                Some(expected.into())
            );
        }
        snapshot.surfaces[1] = PickSurface::Image(alpha_image(1, 1, &[192]));
        assert_eq!(
            snapshot.pick(at(0.6)).unwrap().object_id,
            Some("front".into())
        );
        snapshot.scene.objects[1].material.color.a = 0.8;
        assert_eq!(
            snapshot.pick(at(0.6)).unwrap().object_id,
            Some("rear".into())
        );
        snapshot.scene.objects[1].material.alpha_mode = crate::AlphaMode::Blend;
        assert_eq!(
            snapshot.pick(at(-0.6)).unwrap().object_id,
            Some("front".into())
        );
        snapshot.scene.objects[1].mesh = snapshot.scene.objects[1]
            .mesh
            .with_vertex_colors(vec![[1., 1., 1., 0.]; 3])
            .unwrap();
        assert_eq!(
            snapshot.pick(at(0.6)).unwrap().object_id,
            Some("rear".into())
        );
        snapshot.scene.objects[1].material.alpha_mode = crate::AlphaMode::Opaque;
        assert_eq!(
            snapshot.pick(at(0.6)).unwrap().object_id,
            Some("front".into())
        );
    }

    #[test]
    fn image_cutouts_pick_through_with_bilinear_alpha_and_material_cutoff() {
        let image = alpha_image(2, 2, &[255, 0, 255, 0]);
        let front = Object::new(Mesh::plane(), Material::image(image.clone()))
            .id("front")
            .position([0., 0., 1.])
            .scale([2.; 3]);
        let scene = Scene::new()
            .object(plane().id("rear").scale([4.; 3]))
            .object(front);
        let mut snapshot = PickSnapshot {
            scene,
            bounds: bounds(),
            surfaces: vec![PickSurface::Solid, PickSurface::Image(image)],
        };
        let at = |x, y| project(Camera::default(), bounds(), [x, y, 1.]);
        for y in [-0.4, 0.4] {
            assert_eq!(
                snapshot.pick(at(-0.5, y)).unwrap().object_id,
                Some("front".into())
            );
            assert_eq!(
                snapshot.pick(at(0.5, y)).unwrap().object_id,
                Some("rear".into())
            );
        }
        assert_eq!(
            snapshot.pick(at(0., 0.)).unwrap().object_id,
            Some("front".into())
        );
        snapshot.scene.objects[1].material.color.a = 0.6;
        assert_eq!(
            snapshot.pick(at(-0.25, 0.)).unwrap().object_id,
            Some("rear".into())
        );
        snapshot.scene.objects[1].material.alpha_cutoff = 0.4;
        assert_eq!(
            snapshot.pick(at(-0.25, 0.)).unwrap().object_id,
            Some("front".into())
        );
        snapshot.scene.objects[1].pick_behavior = PickBehavior::Occlude;
        assert!(snapshot.pick(at(-0.5, 0.)).is_none());
        assert_eq!(
            snapshot.pick(at(0.5, 0.)).unwrap().object_id,
            Some("rear".into())
        );
    }

    #[test]
    fn alpha_modes_distinguish_transparent_pixels_from_unavailable_images() {
        let mut snapshot = PickSnapshot {
            scene: Scene::new()
                .object(plane().id("rear"))
                .object(plane().id("front").position([0., 0., 1.])),
            bounds: bounds(),
            surfaces: vec![
                PickSurface::Solid,
                PickSurface::Image(alpha_image(1, 1, &[64])),
            ],
        };
        let p = project(Camera::default(), bounds(), [0.; 3]);
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
        snapshot.scene.objects[1].material.alpha_mode = crate::AlphaMode::Blend;
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("front".into()));
        snapshot.scene.objects[1].material.color.a = 0.;
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
        snapshot.scene.objects[1].material.alpha_mode = crate::AlphaMode::Opaque;
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("front".into()));
        snapshot.surfaces[1] = PickSurface::Absent;
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
        snapshot.surfaces[1] = PickSurface::Image(alpha_image(1, 1, &[0]));
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("front".into()));
        snapshot.scene.objects[1].material.color.a = 1.;
        snapshot.scene.objects[1].material.alpha_mode = crate::AlphaMode::Blend;
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
        snapshot.surfaces[1] = PickSurface::Solid;
        snapshot.scene.objects[1].pick_behavior = PickBehavior::Occlude;
        assert!(snapshot.pick(p).is_none());
        snapshot.scene.objects[1].pick_behavior = PickBehavior::Ignore;
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
    }

    #[test]
    fn missing_sources_and_pick_modes_preserve_depth_order() {
        let scene = Scene::new()
            .object(plane().id("rear"))
            .object(plane().id("front").position([0., 0., 1.]));
        let mut snapshot = PickSnapshot {
            scene,
            bounds: bounds(),
            surfaces: vec![PickSurface::Solid, PickSurface::Absent],
        };
        let p = project(Camera::default(), bounds(), [0.; 3]);
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
        snapshot.surfaces[1] = PickSurface::Image(alpha_image(0, 0, &[]));
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
        snapshot.surfaces[1] = PickSurface::Image(alpha_image(1, 1, &[255]));
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("front".into()));
        snapshot.scene.objects[1].pick_behavior = PickBehavior::Occlude;
        assert!(snapshot.pick(p).is_none());
        snapshot.scene.objects[1].pick_behavior = PickBehavior::Ignore;
        assert_eq!(snapshot.pick(p).unwrap().object_id, Some("rear".into()));
        assert_eq!(
            snapshot.scene.pick(bounds(), p).unwrap().object_id,
            Some("rear".into())
        );
    }

    #[test]
    fn alpha_sampling_clamps_uvs_and_handles_single_pixel_axes() {
        let image = alpha_image(1, 2, &[0, 255]);
        assert_eq!(image_alpha(&image, [-3., -2.], Default::default()), 0.);
        assert_eq!(image_alpha(&image, [5., 2.], Default::default()), 1.);
        assert!((image_alpha(&image, [0.7, 0.375], Default::default()) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn image_sampling_wraps_texel_neighbors_and_applies_uv_transforms() {
        use crate::{TextureAddressMode as Address, TextureFilter, TextureSampling, UvTransform};
        let image = alpha_image(2, 1, &[0, 255]);
        for (mode, u, expected) in [
            (Address::Clamp, 0., 0.),
            (Address::Clamp, 1., 1.),
            (Address::Repeat, 0., 0.5),
            (Address::Repeat, 1., 0.5),
            (Address::Repeat, -0.25, 1.),
            (Address::Repeat, 1.25, 0.),
            (Address::Mirror, -0.25, 0.),
            (Address::Mirror, 1.25, 1.),
            (Address::Mirror, 2., 0.),
        ] {
            let sampling = TextureSampling {
                address_u: mode,
                ..Default::default()
            };
            assert_eq!(image_alpha(&image, [u, 0.5], sampling), expected);
            let vertical = alpha_image(1, 2, &[0, 255]);
            assert_eq!(
                image_alpha(
                    &vertical,
                    [0.5, u],
                    TextureSampling {
                        address_v: mode,
                        ..Default::default()
                    }
                ),
                expected
            );
        }
        for (filter, expected) in [(TextureFilter::Nearest, 1.), (TextureFilter::Linear, 0.75)] {
            assert_eq!(
                image_alpha(
                    &image,
                    [0.625, 0.5],
                    TextureSampling {
                        filter,
                        ..Default::default()
                    }
                ),
                expected
            );
        }
        let transform = UvTransform::from_scale_rotation_translation(
            [2., -1.],
            std::f32::consts::FRAC_PI_2,
            [0.1, 0.2],
        )
        .unwrap();
        let uv = transform.transform([0.2, 0.3]).unwrap();
        assert!((uv[0] - 0.4).abs() < 1e-6 && (uv[1] - 0.6).abs() < 1e-6);
        let collapsed = UvTransform::from_rows([[0., 0., 0.75], [0., 0., 0.5]]).unwrap();
        assert_eq!(
            image_alpha(
                &image,
                [8., -4.],
                TextureSampling {
                    transform: collapsed,
                    ..Default::default()
                }
            ),
            1.
        );
        assert!(UvTransform::from_rows([[1., 0., f32::NAN], [0., 1., 0.]]).is_err());
        assert!(
            UvTransform::from_scale_rotation_translation([1.; 2], f32::INFINITY, [0.; 2]).is_err()
        );
        let overflow = UvTransform::from_rows([[f32::MAX, 0., 0.], [0., 1., 0.]]).unwrap();
        assert_eq!(
            image_alpha(
                &image,
                [2., 0.],
                TextureSampling {
                    transform: overflow,
                    ..Default::default()
                }
            ),
            0.
        );
    }

    #[test]
    fn image_sampling_changes_cutout_hits_without_changing_mesh_uvs() {
        use crate::{TextureFilter, TextureSampling, UvTransform};
        let image = alpha_image(2, 1, &[0, 255]);
        let point = project(Camera::default(), bounds(), [0.; 3]);
        let scene = Scene::new()
            .object(plane().id("rear").position([0., 0., -1.]))
            .object(
                Object::new(
                    Mesh::plane(),
                    Material::image(image.clone()).alpha_cutoff(0.9),
                )
                .id("front"),
            );
        let mut snapshot = PickSnapshot {
            scene,
            bounds: bounds(),
            surfaces: vec![PickSurface::Solid, PickSurface::Image(image)],
        };
        let sampling = TextureSampling {
            transform: UvTransform::from_rows([[0., 0., 0.625], [0., 0., 0.5]]).unwrap(),
            ..Default::default()
        };
        snapshot.scene.objects[1].material = snapshot.scene.objects[1]
            .material
            .clone()
            .image_sampling(sampling);
        assert_eq!(snapshot.pick(point).unwrap().object_id, Some("rear".into()));
        snapshot.scene.objects[1].material.sampling.filter = TextureFilter::Nearest;
        let hit = snapshot.pick(point).unwrap();
        assert_eq!(hit.object_id, Some("front".into()));
        assert_eq!(hit.uv, [0.5, 0.5]);
        snapshot.scene.objects[1].material.sampling.mag_filter = Some(TextureFilter::Linear);
        assert_eq!(snapshot.pick(point).unwrap().object_id, Some("rear".into()));
        snapshot.scene.objects[1].material.sampling.filter = TextureFilter::Linear;
        snapshot.scene.objects[1].material.sampling.mag_filter = Some(TextureFilter::Nearest);
        assert_eq!(
            snapshot.pick(point).unwrap().object_id,
            Some("front".into())
        );
    }

    #[test]
    fn cutout_uses_selected_coordinates_without_changing_geometric_hit_uvs() {
        let image = alpha_image(2, 1, &[0, 255]);
        let mesh = Mesh::plane().with_uv_set(7, vec![[0.25, 0.5]; 4]).unwrap();
        let scene = Scene::new()
            .object(plane().id("rear").position([0., 0., -1.]))
            .object(Object::new(mesh, Material::image(image.clone()).image_uv_set(7)).id("front"));
        let mut snapshot = PickSnapshot {
            scene,
            bounds: bounds(),
            surfaces: vec![PickSurface::Solid, PickSurface::Image(image)],
        };
        let point = project(Camera::default(), bounds(), [0.25, 0., 0.]);
        assert_eq!(snapshot.pick(point).unwrap().object_index, 0);
        let geometric = snapshot.scene.pick(bounds(), point).unwrap();
        assert_eq!(geometric.object_index, 1);
        assert!((geometric.uv[0] - 0.75).abs() < 1e-6);
        let object = &mut snapshot.scene.objects[1];
        object.mesh = object.mesh.with_uv_set(7, vec![[0.75, 0.5]; 4]).unwrap();
        let hit = snapshot.pick(point).unwrap();
        assert_eq!(hit.object_index, 1);
        assert_eq!(hit.uv, geometric.uv);
        snapshot.scene.objects[1].material.uv_set = 9;
        assert_eq!(snapshot.pick(point).unwrap().object_index, 0);
        assert_eq!(
            snapshot.scene.pick(bounds(), point).unwrap().object_index,
            1
        );
    }

    #[test]
    fn nearest_surface_is_order_independent_and_unnamed_objects_occlude() {
        let camera = Camera::default();
        let bounds = bounds();
        let p = project(camera, bounds, [0., 0., 0.]);
        let near = plane().id("near").position([0., 0., 1.]);
        let far = plane().id("far");
        for objects in [[far.clone(), near.clone()], [near, far]] {
            let scene = objects.into_iter().fold(Scene::new(), Scene::object);
            let hit = scene.pick(bounds, p).unwrap();
            assert_eq!(hit.object_id, Some("near".into()));
            assert!((hit.distance - 5.).abs() < 1e-5);
            let blocked = scene.object(plane().position([0., 0., 2.]));
            assert!(blocked.pick(bounds, p).unwrap().object_id.is_none());
        }
    }

    #[test]
    fn transformed_surface_preserves_uv_position_and_backface_normals() {
        let camera = Camera::orbit(0.2, 0.1, 6.);
        let local = [0.21, -0.17, 0., 1.];
        for scale in [[2., 0.7, 1.3], [-2., 0.7, 1.3]] {
            let pose = Transform {
                position: [0.3, 0.2, -0.4],
                rotation: [0.2, -0.3, 0.4],
                scale,
            };
            let (model, normals) = pose.matrices();
            let world = transform(model, local);
            let position = [world[0], world[1], world[2]];
            let scene = Scene::new().camera(camera).object(plane().transform(pose));
            let hit = scene
                .pick(bounds(), project(camera, bounds(), position))
                .unwrap();
            for (actual, expected) in hit.position.into_iter().zip(position) {
                assert!((actual - expected).abs() < 1e-4);
            }
            assert!((hit.uv[0] - 0.71).abs() < 1e-4);
            assert!((hit.uv[1] - 0.67).abs() < 1e-4);
            let n = transform(normals, [0., 0., 1., 0.]);
            let expected = unit([n[0], n[1], n[2]]);
            assert!(dot(hit.normal, expected) > 0.9999);
            assert!(dot(hit.normal, sub(camera.eye, hit.position)) > 0.);
            assert!((hit.barycentric.iter().sum::<f32>() - 1.).abs() < 1e-5);
        }
    }

    #[test]
    fn viewport_offsets_aspect_and_scale_match_projection() {
        let camera = Camera {
            eye: [0., 6., 0.],
            ..Default::default()
        };
        let position = [0.1, 0., 0.2];
        let scene = Scene::new().camera(camera).object(plane().rotation([
            std::f32::consts::FRAC_PI_2,
            0.,
            0.,
        ]));
        for scale in [1., 1.5, 2.] {
            let bounds = Bounds::new(
                bounds().origin * scale,
                size(bounds().size.width * scale, bounds().size.height * scale),
            );
            let hit = scene
                .pick(bounds, project(camera, bounds, position))
                .unwrap();
            for (actual, expected) in hit.position.into_iter().zip(position) {
                assert!((actual - expected).abs() < 1e-4);
            }
            assert!(
                scene
                    .pick(bounds, bounds.origin - point(px(1.), px(1.)))
                    .is_none()
            );
            assert!(scene.pick(bounds, bounds.bottom_right()).is_none());
        }
    }

    #[test]
    fn clip_planes_use_view_depth_and_reject_transparent_or_degenerate_geometry() {
        let camera = Camera {
            eye: [0., 0., 0.],
            target: [0., 0., -1.],
            near: 1.,
            far: 3.,
            projection: crate::Projection::Perspective { vertical_fov: 2. },
            ..Default::default()
        };
        let bounds = bounds();
        let p = project(camera, bounds, [1.2, 0., -0.9]);
        let clipped = Scene::new()
            .camera(camera)
            .object(plane().position([0., 0., -0.9]).scale([8.; 3]))
            .object(plane().position([0., 0., -3.1]).scale([16.; 3]));
        assert!(clipped.pick(bounds, p).is_none());
        let transparent = Object::new(Mesh::plane(), Material::color(gpui::rgba(0xffffff10)))
            .position([0., 0., -1.1])
            .scale([8.; 3]);
        let vertices = vec![
            crate::Vertex {
                position: [0., 0., -1.5],
                normal: [0., 0., 1.],
                uv: [0.; 2]
            };
            3
        ];
        let degenerate = Object::new(
            Mesh::new(vertices, vec![0, 1, 2]),
            Material::color(rgb(0xffffff)),
        );
        let scene = clipped
            .object(plane().id("surface").position([0., 0., -2.]).scale([8.; 3]))
            .object(transparent)
            .object(degenerate);
        assert_eq!(
            scene.pick(bounds, p).unwrap().object_id,
            Some("surface".into())
        );
        assert!(scene.pick(Bounds::default(), p).is_none());
        assert!(scene.pick(bounds, point(px(f32::NAN), px(0.))).is_none());
    }
}
