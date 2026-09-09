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
        self.scene
            .pick_filtered(self.bounds, position, |index, uv| {
                match &self.surfaces[index] {
                    PickSurface::Absent => 0.,
                    PickSurface::Solid => 1.,
                    PickSurface::Image(image) => image_alpha(image, uv),
                }
            })
    }
}

fn image_alpha(image: &RenderImage, uv: [f32; 2]) -> f32 {
    let size = image.size(0);
    let Some(bytes) = image.as_bytes(0) else {
        return 0.;
    };
    if size.width.0 <= 0 || size.height.0 <= 0 || !uv.iter().all(|v| v.is_finite()) {
        return 0.;
    }
    let width = size.width.0 as usize;
    let height = size.height.0 as usize;
    let x = uv[0].clamp(0., 1.) * (width - 1) as f32;
    let y = uv[1].clamp(0., 1.) * (height - 1) as f32;
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let alpha = |x, y| f32::from(bytes[(y * width + x) * 4 + 3]) / 255.;
    let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
    mix(
        mix(alpha(x0, y0), alpha(x1, y0), x.fract()),
        mix(alpha(x0, y1), alpha(x1, y1), x.fract()),
        y.fract(),
    )
}

/// The nearest triangle intersection within the camera's clip range.
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
    /// Interpolated texture coordinates; `(0, 0)` is the top left.
    pub uv: [f32; 2],
    /// Weights corresponding to the triangle's three indexed vertices.
    pub barycentric: [f32; 3],
    /// World-space distance from the ray origin. Orthographic camera rays start
    /// on the eye plane, so this is forward depth rather than distance to the eye.
    pub distance: f32,
}

impl Scene {
    /// Finds the nearest geometric surface at a logical window position.
    ///
    /// Both faces are tested. Unnamed objects still occlude other objects.
    /// Constant material alpha is respected; texture alpha, image availability,
    /// ancestor clipping, and effect deformation are not sampled by this query.
    /// Viewport callbacks also sample prepared image alpha and use GPUI hitbox routing.
    pub fn pick(&self, bounds: Bounds<Pixels>, position: Point<Pixels>) -> Option<Hit> {
        self.pick_filtered(bounds, position, |_, _| 1.)
    }

    /// Geometric world-ray query, independent of the scene camera and its clip range.
    /// Respects material alpha and picking behavior, but does not resolve image alpha.
    pub fn raycast(&self, ray: crate::Ray) -> Option<Hit> {
        self.trace(ray, |_| true, |_, _| 1.)
    }

    fn pick_filtered(
        &self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        alpha: impl Fn(usize, [f32; 2]) -> f32,
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
        )
    }

    fn trace(
        &self,
        ray: crate::Ray,
        within: impl Fn([f32; 3]) -> bool,
        alpha: impl Fn(usize, [f32; 2]) -> f32,
    ) -> Option<Hit> {
        let direction = ray.direction();
        let mut closest: Option<Hit> = None;
        for (object_index, object) in self.objects.iter().enumerate() {
            if object.pick_behavior == PickBehavior::Ignore
                || object.material.color.a < object.material.alpha_cutoff
            {
                continue;
            }
            let (model, normal_matrix) = object.matrices();
            for (triangle_index, indices) in object.mesh.0.indices().chunks_exact(3).enumerate() {
                let vertices: [_; 3] =
                    std::array::from_fn(|i| &object.mesh.0.vertices()[indices[i] as usize]);
                let world = vertices.map(|v| {
                    let p = transform(model, [v.position[0], v.position[1], v.position[2], 1.]);
                    [p[0], p[1], p[2]]
                });
                let Some((distance, barycentric, front)) =
                    intersect(ray.origin(), direction, world, true)
                else {
                    continue;
                };
                if closest.as_ref().is_some_and(|hit| distance >= hit.distance) {
                    continue;
                }
                let position = ray.at(distance);
                if !within(position) {
                    continue;
                }
                let uv = std::array::from_fn(|i| {
                    (0..3).map(|j| vertices[j].uv[i] * barycentric[j]).sum()
                });
                if alpha(object_index, uv) * object.material.color.a < object.material.alpha_cutoff
                {
                    continue;
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
            }
        }
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

    fn alpha_image(width: u32, height: u32, alpha: &[u8]) -> Arc<RenderImage> {
        let bytes = alpha.iter().flat_map(|a| [255, 255, 255, *a]).collect();
        Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::from_raw(width, height, bytes).unwrap(),
        )]))
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
            snapshot.pick(at(-0.5, 0.)).unwrap().object_id,
            Some("rear".into())
        );
        snapshot.scene.objects[1].material.alpha_cutoff = 0.4;
        assert_eq!(
            snapshot.pick(at(-0.5, 0.)).unwrap().object_id,
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
        assert_eq!(image_alpha(&image, [-3., -2.]), 0.);
        assert_eq!(image_alpha(&image, [5., 2.]), 1.);
        assert!((image_alpha(&image, [0.7, 0.25]) - 0.25).abs() < 1e-6);
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
            let expected = unit([n[0], n[1], n[2]]).map(|n| if scale[0] < 0. { -n } else { n });
            assert!(dot(hit.normal, expected) > 0.9999);
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
