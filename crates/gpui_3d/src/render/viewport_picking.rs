use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::Arc,
};

use anyhow::Result;
use gpui::{Bounds, Pixels, Point, Scene3dFrame, Scene3dPickCapture, Size};
use gpui_wgpu::WgpuScene3dPickFrame;

use crate::{Camera, FramePick, FramePickReadback, PreparedScene, RenderObject};

#[derive(Clone, Copy, PartialEq)]
pub(super) struct PickLayout {
    pub bounds: Bounds<Pixels>,
    pub scale: f32,
    pub surface: Size<Pixels>,
}

impl PickLayout {
    fn surface_region(&self, selection: Bounds<Pixels>) -> Option<[f32; 4]> {
        let rect = |bounds: Bounds<Pixels>| {
            [
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            ]
        };
        let viewport = rect(self.bounds);
        let selection = rect(selection);
        if !self.scale.is_finite()
            || self.scale <= 0.
            || viewport.iter().chain(&selection).any(|v| !v.is_finite())
        {
            return None;
        }
        let mut surface = [0.; 4];
        for axis in 0..2 {
            if viewport[axis + 2] <= 0. || selection[axis + 2] <= 0. {
                return None;
            }
            let start = f64::from(selection[axis]).max(f64::from(viewport[axis]));
            let end = (f64::from(selection[axis]) + f64::from(selection[axis + 2]))
                .min(f64::from(viewport[axis]) + f64::from(viewport[axis + 2]));
            if end <= start {
                return None;
            }
            surface[axis] = (start * f64::from(self.scale)) as f32;
            surface[axis + 2] = ((end - start) * f64::from(self.scale)) as f32;
        }
        surface.iter().all(|v| v.is_finite()).then_some(surface)
    }
}

struct Snapshot {
    frame: Arc<Scene3dFrame>,
    objects: Arc<[RenderObject]>,
    camera: Camera,
    layout: PickLayout,
    owner: Weak<()>,
}

/// Retained ID/depth capture for one viewport. Attach the same handle on each render.
/// Requests use logical coordinates in the viewport's input space, before outer effects.
#[derive(Clone)]
pub struct ViewportPickCapture {
    backend: Scene3dPickCapture,
    snapshot: Rc<RefCell<Option<Rc<Snapshot>>>>,
}

impl ViewportPickCapture {
    /// Sets the per-frame output/attachment payload limit, excluding retained older frames.
    pub fn new(max_bytes: u64) -> Self {
        Self {
            backend: Scene3dPickCapture::new(max_bytes),
            snapshot: Rc::default(),
        }
    }

    /// Latest submitted output matching the mounted viewport's frame and layout.
    /// `None` means not painted/submitted yet, removed, or awaiting a matching render.
    pub fn frame(&self) -> Result<Option<ViewportPickFrame>> {
        let Some(snapshot) = self
            .snapshot
            .borrow()
            .clone()
            .filter(|s| s.owner.upgrade().is_some())
        else {
            return Ok(None);
        };
        let Some(output) = self
            .backend
            .read_for_frame::<WgpuScene3dPickFrame>(&snapshot.frame)
        else {
            return Ok(None);
        };
        let output = output.map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(output
            .matches_frame(&snapshot.frame)
            .then_some(ViewportPickFrame { snapshot, output }))
    }

    /// Requests one pixel from the current submitted frame. `None` means no matching
    /// frame or a position outside the viewport/surface. Backend failures remain errors.
    pub fn pick(&self, position: Point<Pixels>) -> Result<Option<ViewportPickReadback>> {
        self.frame()?
            .map(|frame| frame.pick(position))
            .transpose()
            .map(Option::flatten)
    }

    /// Whether a result still belongs to the latest mounted, submitted frame and layout.
    /// A false value does not invalidate the retained result's original object identities.
    pub fn is_current(&self, result: &ViewportPick) -> bool {
        self.frame().ok().flatten().is_some_and(|frame| {
            Rc::downgrade(&frame.snapshot).ptr_eq(&result.snapshot)
                && Arc::downgrade(&frame.output).ptr_eq(&result.output)
        })
    }

    /// Checks an output identity before or after asynchronous readback. Missing,
    /// removed, failed or not-yet-submitted viewport bindings return false.
    pub fn is_current_frame(&self, frame_id: &crate::Scene3dFrameId) -> bool {
        self.frame()
            .ok()
            .flatten()
            .is_some_and(|frame| frame.frame_id() == frame_id)
    }

    /// Releases the current binding without invalidating retained frames or requests.
    pub fn clear(&self) {
        *self.snapshot.borrow_mut() = None;
    }

    pub(super) fn backend(&self) -> Scene3dPickCapture {
        self.backend.clone()
    }

    pub(super) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.snapshot, &other.snapshot)
    }

    pub(super) fn bind(
        &self,
        frame: Arc<Scene3dFrame>,
        prepared: &PreparedScene,
        mut camera: Camera,
        layout: PickLayout,
        owner: &Rc<()>,
    ) {
        let mut current = self.snapshot.borrow_mut();
        if current.as_ref().is_some_and(|old| {
            Arc::ptr_eq(&old.frame, &frame)
                && old.layout == layout
                && old.owner.ptr_eq(&Rc::downgrade(owner))
        }) {
            return;
        }
        camera.aspect_ratio.get_or_insert(
            f32::from(layout.bounds.size.width) / f32::from(layout.bounds.size.height),
        );
        *current = Some(Rc::new(Snapshot {
            frame,
            objects: prepared.identities(),
            camera,
            layout,
            owner: Rc::downgrade(owner),
        }));
    }
}

/// One submitted viewport output with its source camera, object mapping, and layout.
#[derive(Clone)]
pub struct ViewportPickFrame {
    snapshot: Rc<Snapshot>,
    output: Arc<WgpuScene3dPickFrame>,
}

impl ViewportPickFrame {
    /// Physical dimensions of this capture's ID/depth textures.
    pub fn size(&self) -> [u32; 2] {
        self.output.gpu().config().size
    }

    /// Reads a rectangle in capture-texture pixels with the original projection and identities.
    /// Available channels are ID and linear depth. Use the request's frame ID for freshness.
    pub fn readback_region(
        &self,
        region: crate::Scene3dReadbackRegion,
        config: crate::Scene3dReadbackConfig,
    ) -> Result<crate::FrameReadback> {
        crate::FrameReadback::new(
            self.output.gpu(),
            self.snapshot.objects.clone(),
            self.snapshot.camera,
            region,
            config,
            Some(self.output.projection_rect()),
        )
    }
    /// Identity of the submitted ID/depth output, not only the prepared scene.
    pub fn frame_id(&self) -> &crate::Scene3dFrameId {
        self.output.gpu().frame_id()
    }
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.snapshot.layout.bounds
    }

    /// Maps logical viewport-input coordinates to a capture pixel without starting readback.
    /// Outside, nonfinite and surface-clipped positions return None.
    pub fn pixel_at(&self, position: Point<Pixels>) -> Option<[u32; 2]> {
        normalized(self.bounds(), position)?;
        self.output.pixel_at_surface([
            f32::from(position.x) * self.snapshot.layout.scale,
            f32::from(position.y) * self.snapshot.layout.scale,
        ])
    }

    /// Maps a logical input rectangle to an outward-rounded capture-pixel region.
    /// Clips to this frame's viewport and render surface. Empty, nonfinite or fully
    /// clipped rectangles return None. Border pixels may only partially intersect
    /// the rectangle; outer effects and UI hit eligibility remain caller-owned.
    /// Does not allocate resources or start a readback.
    pub fn region_at(&self, bounds: Bounds<Pixels>) -> Option<crate::Scene3dReadbackRegion> {
        self.output
            .region_at_surface(self.snapshot.layout.surface_region(bounds)?)
    }

    /// Retains the selected frame even if subsequent viewport renders replace it.
    pub fn pick(&self, position: Point<Pixels>) -> Result<Option<ViewportPickReadback>> {
        let Some(pixel) = self.pixel_at(position) else {
            return Ok(None);
        };
        Ok(Some(ViewportPickReadback {
            pending: FramePickReadback::new(
                self.output.gpu(),
                self.snapshot.objects.clone(),
                self.snapshot.camera,
                pixel,
                Some(self.output.projection_rect()),
            )?,
            snapshot: self.snapshot.clone(),
            output: self.output.clone(),
            position,
        }))
    }
}

/// Completed viewport query. Object identity and world position belong to the source frame.
#[derive(Clone)]
pub struct ViewportPick {
    pub frame: FramePick,
    pub position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    snapshot: Weak<Snapshot>,
    output: std::sync::Weak<WgpuScene3dPickFrame>,
}

impl ViewportPick {
    pub fn frame_id(&self) -> &crate::Scene3dFrameId {
        self.frame.frame_id()
    }
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.bounds
    }
}

/// Nonblocking viewport request. Dropping it cancels mapping, not submitted GPU work.
pub struct ViewportPickReadback {
    pending: FramePickReadback,
    snapshot: Rc<Snapshot>,
    output: Arc<WgpuScene3dPickFrame>,
    position: Point<Pixels>,
}

impl ViewportPickReadback {
    pub fn frame_id(&self) -> &crate::Scene3dFrameId {
        self.pending.frame_id()
    }
    /// `None` means pending; completion and errors are terminal. No redraws are scheduled.
    pub fn try_read(&mut self) -> Result<Option<ViewportPick>> {
        Ok(self.pending.try_read()?.map(|frame| ViewportPick {
            frame,
            position: self.position,
            bounds: self.snapshot.layout.bounds,
            snapshot: Rc::downgrade(&self.snapshot),
            output: Arc::downgrade(&self.output),
        }))
    }
}

fn normalized(bounds: Bounds<Pixels>, position: Point<Pixels>) -> Option<[f32; 2]> {
    let uv = [
        f32::from(position.x - bounds.origin.x) / f32::from(bounds.size.width),
        f32::from(position.y - bounds.origin.y) / f32::from(bounds.size.height),
    ];
    (!bounds.is_empty() && uv.iter().all(|v| v.is_finite() && (0. ..1.).contains(v))).then_some(uv)
}

#[cfg(test)]
mod tests;
