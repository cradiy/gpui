use anyhow::{Context, Result, ensure};
use gpui_3d::{Camera, Projection};

use crate::PreparedDocument;

impl PreparedDocument {
    /// Converts a camera in local coordinates: origin eye, -Z viewing direction,
    /// +Y up. Scene nodes supply its world transform; selection remains explicit.
    pub fn camera(&self, index: usize) -> Result<Camera> {
        self.convert_camera(index)
            .with_context(|| format!("camera {index}"))
    }

    fn convert_camera(&self, index: usize) -> Result<Camera> {
        crate::validation::supported_extensions(self.gltf())?;
        let source = self
            .gltf()
            .cameras()
            .nth(index)
            .context("camera index out of range")?;
        let mut camera = Camera {
            eye: [0.; 3],
            target: [0., 0., -1.],
            up: [0., 1., 0.],
            ..Default::default()
        };
        match source.projection() {
            gltf::camera::Projection::Perspective(p) => {
                ensure!(
                    p.zfar().is_none_or(f32::is_finite),
                    "non-finite explicit zfar"
                );
                camera.projection = Projection::Perspective {
                    vertical_fov: p.yfov(),
                };
                camera.aspect_ratio = p.aspect_ratio();
                camera.near = p.znear();
                camera.far = p.zfar().unwrap_or(f32::INFINITY);
            }
            gltf::camera::Projection::Orthographic(p) => {
                ensure!(
                    p.xmag().is_finite() && p.xmag() > 0. && p.ymag().is_finite() && p.ymag() > 0.,
                    "orthographic magnitudes must be finite and positive"
                );
                ensure!(
                    p.znear() > 0.,
                    "orthographic znear must be positive; zero near depth is unsupported by the core"
                );
                camera.projection = Projection::Orthographic {
                    vertical_size: 2. * p.ymag(),
                };
                camera.aspect_ratio = Some(p.xmag() / p.ymag());
                camera.near = p.znear();
                camera.far = p.zfar();
            }
        }
        camera
            .view_projection(1.)
            .context("invalid camera parameters")?;
        Ok(camera)
    }
}
