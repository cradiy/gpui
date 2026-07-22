#[cfg(target_os = "linux")]
mod linux {
    use std::{
        ffi::c_void,
        fs::{self, File},
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
        path::{Path, PathBuf},
        ptr,
        sync::Arc,
    };

    use anyhow::{Context as _, Result, anyhow, bail};
    use gpui::{
        App, Bounds, ColorRange, Context, DRM_FORMAT_NV12, DevicePixels, DmaBufHandle, DmaBufImage,
        DmaBufObject, DmaBufPlane, DmaBufPlaneLayout, GpuSpecs, Render, SurfaceColorInfo,
        SurfaceFormat, SurfaceFrame, SurfaceHandle, Window, WindowBounds, WindowOptions, YuvMatrix,
        bounds, div, prelude::*, px, size, surface,
    };
    use gpui_platform::application;

    const WIDTH: u32 = 960;
    const HEIGHT: u32 = 540;
    const DRM_FORMAT_ARGB8888: u32 = fourcc(b'A', b'R', b'2', b'4');
    const DRM_FORMAT_R8: u32 = fourcc(b'R', b'8', b' ', b' ');
    const DRM_FORMAT_GR88: u32 = fourcc(b'G', b'R', b'8', b'8');
    const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
    const DRM_FORMAT_MOD_LINEAR: u64 = 0;
    const GBM_BO_USE_RENDERING: u32 = 1 << 2;
    const GBM_BO_USE_LINEAR: u32 = 1 << 4;
    const GBM_BO_TRANSFER_WRITE: u32 = 1 << 1;
    const DMA_BUF_IOCTL_SYNC: libc::c_ulong = 0x4008_6200;
    const DMA_BUF_SYNC_WRITE: u64 = 2;
    const DMA_BUF_SYNC_END: u64 = 1 << 2;

    const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
        a as u32 | (b as u32) << 8 | (c as u32) << 16 | (d as u32) << 24
    }

    #[repr(C)]
    struct GbmDevice {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct GbmBo {
        _private: [u8; 0],
    }

    #[repr(C)]
    struct DmaBufSync {
        flags: u64,
    }

    #[link(name = "gbm")]
    unsafe extern "C" {
        fn gbm_create_device(fd: i32) -> *mut GbmDevice;
        fn gbm_device_destroy(device: *mut GbmDevice);
        fn gbm_device_is_format_supported(device: *mut GbmDevice, format: u32, flags: u32) -> i32;
        fn gbm_bo_create_with_modifiers2(
            device: *mut GbmDevice,
            width: u32,
            height: u32,
            format: u32,
            modifiers: *const u64,
            modifier_count: u32,
            flags: u32,
        ) -> *mut GbmBo;
        fn gbm_bo_create(
            device: *mut GbmDevice,
            width: u32,
            height: u32,
            format: u32,
            flags: u32,
        ) -> *mut GbmBo;
        fn gbm_bo_map(
            bo: *mut GbmBo,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
            flags: u32,
            stride: *mut u32,
            map_data: *mut *mut c_void,
        ) -> *mut c_void;
        fn gbm_bo_unmap(bo: *mut GbmBo, map_data: *mut c_void);
        fn gbm_bo_get_stride_for_plane(bo: *mut GbmBo, plane: i32) -> u32;
        fn gbm_bo_get_offset(bo: *mut GbmBo, plane: i32) -> u32;
        fn gbm_bo_get_fd_for_plane(bo: *mut GbmBo, plane: i32) -> i32;
        fn gbm_bo_get_modifier(bo: *mut GbmBo) -> u64;
        fn gbm_bo_get_plane_count(bo: *mut GbmBo) -> i32;
        fn gbm_bo_destroy(bo: *mut GbmBo);
    }

    struct GbmAllocation {
        bos: Vec<*mut GbmBo>,
        device: *mut GbmDevice,
        _render_node: File,
    }

    // The allocation is immutable after construction. GPUI retains this guard until the GPU
    // submission completes, and the final reference only destroys the GBM objects once.
    unsafe impl Send for GbmAllocation {}
    unsafe impl Sync for GbmAllocation {}

    impl Drop for GbmAllocation {
        fn drop(&mut self) {
            unsafe {
                for bo in self.bos.drain(..) {
                    gbm_bo_destroy(bo);
                }
                gbm_device_destroy(self.device);
            }
        }
    }

    struct ImportedFrame {
        frame: Arc<SurfaceFrame>,
        render_node: PathBuf,
        layout: String,
    }

    struct DmaBufExample {
        handle: SurfaceHandle,
        imported: Option<ImportedFrame>,
        error: Option<String>,
        attempted: bool,
    }

    impl DmaBufExample {
        fn initialize(&mut self, specs: &GpuSpecs) {
            self.attempted = true;
            match create_frame(self.handle.clone(), specs) {
                Ok(imported) => self.imported = Some(imported),
                Err(error) => self.error = Some(format!("{error:#}")),
            }
        }
    }

    impl Render for DmaBufExample {
        fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let specs = window.gpu_specs();
            if !self.attempted
                && let Some(specs) = specs.as_ref()
            {
                self.initialize(specs);
            }

            let device_status = specs
                .as_ref()
                .map(|specs| {
                    format!(
                        "GPU: {} · DMA-BUF import: {} · native NV12: {}",
                        specs.device_name,
                        if specs.supports_dma_buf_import {
                            "available"
                        } else {
                            "unavailable"
                        },
                        if specs.supports_native_nv12_dma_buf_import {
                            format!(
                                "available ({} modifiers)",
                                specs.native_nv12_dma_buf_modifiers.len()
                            )
                        } else {
                            "unavailable".to_string()
                        },
                    )
                })
                .unwrap_or_else(|| "Waiting for GPU information".to_string());

            let mut root = div()
                .flex()
                .flex_col()
                .size_full()
                .gap_3()
                .p_6()
                .bg(gpui::rgb(0x11151b))
                .text_color(gpui::white())
                .child("Linux GBM → DMA-BUF → Vulkan texture")
                .child(device_status);

            if let Some(imported) = &self.imported {
                root = root
                    .child(format!(
                        "Node: {} · {}",
                        imported.render_node.display(),
                        imported.layout,
                    ))
                    .child(
                        surface(imported.frame.clone())
                            .w_full()
                            .h(px(540.0))
                            .rounded_xl()
                            .overflow_hidden(),
                    );
            } else if let Some(error) = &self.error {
                root = root.child(format!("DMA-BUF setup failed: {error}"));
            }

            root
        }
    }

    fn create_frame(handle: SurfaceHandle, specs: &GpuSpecs) -> Result<ImportedFrame> {
        if !specs.supports_dma_buf_import {
            bail!("the selected GPUI adapter does not support DMA-BUF import");
        }

        let render_node = find_render_node(specs)?;
        let render_node_file = File::options()
            .read(true)
            .write(true)
            .open(&render_node)
            .with_context(|| format!("failed to open {}", render_node.display()))?;
        let device = unsafe { gbm_create_device(render_node_file.as_raw_fd()) };
        if device.is_null() {
            bail!("gbm_create_device failed for {}", render_node.display());
        }

        match std::env::var("GPUI_DMA_BUF_FORMAT").as_deref() {
            Ok("native-nv12") => {
                if !specs.supports_native_nv12_dma_buf_import {
                    unsafe { gbm_device_destroy(device) };
                    bail!("the selected GPUI adapter does not support native NV12 import");
                }
                return create_native_nv12_frame(
                    handle,
                    render_node,
                    render_node_file,
                    device,
                    specs,
                );
            }
            Ok("nv12") => {
                return create_nv12_frame(handle, render_node, render_node_file, device);
            }
            _ => {}
        }

        let flags = GBM_BO_USE_RENDERING | GBM_BO_USE_LINEAR;

        let modifier = DRM_FORMAT_MOD_LINEAR;
        let mut bo = unsafe {
            gbm_bo_create_with_modifiers2(
                device,
                WIDTH,
                HEIGHT,
                DRM_FORMAT_ARGB8888,
                &modifier,
                1,
                flags,
            )
        };
        let mut has_explicit_linear_layout = !bo.is_null();
        if bo.is_null() {
            bo = unsafe { gbm_bo_create(device, WIDTH, HEIGHT, DRM_FORMAT_ARGB8888, flags) };
            has_explicit_linear_layout = !bo.is_null();
        }
        if bo.is_null()
            && unsafe {
                gbm_device_is_format_supported(device, DRM_FORMAT_ARGB8888, GBM_BO_USE_LINEAR) != 0
            }
        {
            bo = unsafe {
                gbm_bo_create(
                    device,
                    WIDTH,
                    HEIGHT,
                    DRM_FORMAT_ARGB8888,
                    GBM_BO_USE_LINEAR,
                )
            };
            has_explicit_linear_layout = !bo.is_null();
        }
        if bo.is_null()
            && unsafe {
                gbm_device_is_format_supported(device, DRM_FORMAT_ARGB8888, GBM_BO_USE_RENDERING)
                    != 0
            }
        {
            bo = unsafe {
                gbm_bo_create(
                    device,
                    WIDTH,
                    HEIGHT,
                    DRM_FORMAT_ARGB8888,
                    GBM_BO_USE_RENDERING,
                )
            };
        }
        if bo.is_null() {
            unsafe { gbm_device_destroy(device) };
            bail!("GBM failed to allocate an ARGB8888 buffer");
        }

        let allocation = Arc::new(GbmAllocation {
            bos: vec![bo],
            device,
            _render_node: render_node_file,
        });
        paint_test_pattern(bo)?;

        let plane_count = unsafe { gbm_bo_get_plane_count(bo) };
        if plane_count != 1 {
            bail!("expected one GBM plane, got {plane_count}");
        }
        let raw_fd = unsafe { gbm_bo_get_fd_for_plane(bo, 0) };
        if raw_fd < 0 {
            bail!("GBM failed to export the buffer as a DMA-BUF fd");
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        let stride = unsafe { gbm_bo_get_stride_for_plane(bo, 0) };
        let offset = u64::from(unsafe { gbm_bo_get_offset(bo, 0) });
        let reported_modifier = unsafe { gbm_bo_get_modifier(bo) };
        let actual_modifier = match (reported_modifier, has_explicit_linear_layout) {
            (DRM_FORMAT_MOD_INVALID, true) => DRM_FORMAT_MOD_LINEAR,
            (DRM_FORMAT_MOD_INVALID, false) => {
                bail!(
                    "GBM did not report the layout modifier for a non-linear allocation; cannot import it safely"
                )
            }
            (modifier, _) => modifier,
        };
        let coded_size = size(DevicePixels(WIDTH as i32), DevicePixels(HEIGHT as i32));
        let dma_buf = unsafe {
            DmaBufHandle::new_with_lifetime_guard(
                fd,
                coded_size,
                SurfaceFormat::Bgra8,
                actual_modifier,
                offset,
                stride,
                allocation,
            )
        }?;
        let frame = SurfaceFrame::from_dma_buf(
            handle,
            0,
            bounds(Default::default(), coded_size),
            coded_size,
            dma_buf,
        )?;

        Ok(ImportedFrame {
            frame: Arc::new(frame),
            render_node,
            layout: format!("BGRA · modifier: {actual_modifier:#018x} · stride: {stride}"),
        })
    }

    fn create_nv12_frame(
        handle: SurfaceHandle,
        render_node: PathBuf,
        render_node_file: File,
        device: *mut GbmDevice,
    ) -> Result<ImportedFrame> {
        let y_bo = match allocate_linear_bo(device, WIDTH, HEIGHT, DRM_FORMAT_R8) {
            Ok(bo) => bo,
            Err(error) => {
                unsafe { gbm_device_destroy(device) };
                return Err(error).context("GBM failed to allocate the NV12 Y plane");
            }
        };
        let uv_bo = match allocate_linear_bo(
            device,
            WIDTH.div_ceil(2),
            HEIGHT.div_ceil(2),
            DRM_FORMAT_GR88,
        ) {
            Ok(bo) => bo,
            Err(error) => {
                unsafe {
                    gbm_bo_destroy(y_bo);
                    gbm_device_destroy(device);
                }
                return Err(error).context("GBM failed to allocate the NV12 UV plane");
            }
        };
        let allocation = Arc::new(GbmAllocation {
            bos: vec![y_bo, uv_bo],
            device,
            _render_node: render_node_file,
        });

        paint_nv12_y_plane(y_bo)?;
        paint_nv12_uv_plane(uv_bo)?;
        let y_plane = export_linear_plane(y_bo)?;
        let uv_plane = export_linear_plane(uv_bo)?;
        let y_stride = y_plane.stride();
        let uv_stride = uv_plane.stride();
        let y_modifier = y_plane.drm_modifier();
        let uv_modifier = uv_plane.drm_modifier();
        let coded_size = size(DevicePixels(WIDTH as i32), DevicePixels(HEIGHT as i32));
        let dma_buf = unsafe {
            DmaBufHandle::new_nv12_with_lifetime_guard(coded_size, y_plane, uv_plane, allocation)
        }?;
        let color = SurfaceColorInfo {
            matrix: YuvMatrix::Bt709,
            range: ColorRange::Limited,
        };
        let frame = SurfaceFrame::from_dma_buf_with_color(
            handle,
            0,
            bounds(Default::default(), coded_size),
            coded_size,
            dma_buf,
            color,
        )?;

        Ok(ImportedFrame {
            frame: Arc::new(frame),
            render_node,
            layout: format!(
                "NV12 · Y modifier: {y_modifier:#018x}, stride: {y_stride} · UV modifier: {uv_modifier:#018x}, stride: {uv_stride}"
            ),
        })
    }

    fn create_native_nv12_frame(
        handle: SurfaceHandle,
        render_node: PathBuf,
        render_node_file: File,
        device: *mut GbmDevice,
        specs: &GpuSpecs,
    ) -> Result<ImportedFrame> {
        let requested_modifier = requested_native_nv12_modifier()?;
        let modifier_capability = specs
            .native_nv12_dma_buf_modifiers
            .iter()
            .find(|candidate| candidate.modifier == requested_modifier)
            .copied()
            .with_context(|| {
                format!("Vulkan does not advertise native NV12 modifier {requested_modifier:#018x}")
            })?;
        let bo = match allocate_native_nv12_bo(device, requested_modifier) {
            Ok(allocation) => allocation,
            Err(error) => {
                unsafe { gbm_device_destroy(device) };
                return Err(error).with_context(|| {
                    format!(
                        "Vulkan advertises modifier {requested_modifier:#018x} with {} memory planes, but GBM failed to allocate it",
                        modifier_capability.plane_count
                    )
                });
            }
        };
        let allocation = Arc::new(GbmAllocation {
            bos: vec![bo],
            device,
            _render_node: render_node_file,
        });

        let plane_count = unsafe { gbm_bo_get_plane_count(bo) };
        if plane_count != 2 {
            bail!("expected two native NV12 planes, got {plane_count}");
        }
        let y_stride = unsafe { gbm_bo_get_stride_for_plane(bo, 0) };
        let uv_stride = unsafe { gbm_bo_get_stride_for_plane(bo, 1) };
        let y_offset = u64::from(unsafe { gbm_bo_get_offset(bo, 0) });
        let uv_offset = u64::from(unsafe { gbm_bo_get_offset(bo, 1) });

        let y_raw_fd = unsafe { gbm_bo_get_fd_for_plane(bo, 0) };
        let uv_raw_fd = unsafe { gbm_bo_get_fd_for_plane(bo, 1) };
        if y_raw_fd < 0 || uv_raw_fd < 0 {
            if y_raw_fd >= 0 {
                drop(unsafe { OwnedFd::from_raw_fd(y_raw_fd) });
            }
            if uv_raw_fd >= 0 {
                drop(unsafe { OwnedFd::from_raw_fd(uv_raw_fd) });
            }
            bail!("GBM failed to export the native NV12 planes");
        }
        let y_fd = unsafe { OwnedFd::from_raw_fd(y_raw_fd) };
        let uv_fd = unsafe { OwnedFd::from_raw_fd(uv_raw_fd) };
        if dma_buf_identity(&y_fd)? != dma_buf_identity(&uv_fd)? {
            bail!("GBM exported native NV12 as multiple objects; this example expects one");
        }
        drop(uv_fd);
        let modifier = match unsafe { gbm_bo_get_modifier(bo) } {
            DRM_FORMAT_MOD_INVALID => requested_modifier,
            modifier => modifier,
        };
        if modifier != requested_modifier {
            bail!("GBM allocated modifier {modifier:#018x}, requested {requested_modifier:#018x}");
        }
        if modifier == DRM_FORMAT_MOD_LINEAR {
            paint_native_nv12(&y_fd, y_offset, y_stride, uv_offset, uv_stride)?;
        }
        let coded_size = size(DevicePixels(WIDTH as i32), DevicePixels(HEIGHT as i32));
        let mut image = DmaBufImage::new(
            coded_size,
            DRM_FORMAT_NV12,
            vec![DmaBufObject::new(y_fd, modifier)],
            vec![
                DmaBufPlaneLayout::new(0, y_offset, y_stride),
                DmaBufPlaneLayout::new(0, uv_offset, uv_stride),
            ],
        );
        if let Some(drm_device) = specs.drm_render_device {
            image = image.with_drm_device(drm_device);
        }
        let dma_buf = unsafe { DmaBufHandle::from_image_with_lifetime_guard(image, allocation) }?;
        let frame = SurfaceFrame::from_dma_buf_with_color(
            handle,
            0,
            bounds(Default::default(), coded_size),
            coded_size,
            dma_buf,
            SurfaceColorInfo {
                matrix: YuvMatrix::Bt709,
                range: ColorRange::Limited,
            },
        )?;

        Ok(ImportedFrame {
            frame: Arc::new(frame),
            render_node,
            layout: format!(
                "native NV12 · one object / two planes · modifier: {modifier:#018x} · strides: {y_stride}/{uv_stride}"
            ),
        })
    }

    fn requested_native_nv12_modifier() -> Result<u64> {
        let modifier = std::env::var("GPUI_DMA_BUF_MODIFIER")
            .ok()
            .map(|value| {
                u64::from_str_radix(value.trim().trim_start_matches("0x"), 16)
                    .context("invalid GPUI_DMA_BUF_MODIFIER")
            })
            .transpose()?
            .unwrap_or(DRM_FORMAT_MOD_LINEAR);
        Ok(modifier)
    }

    fn allocate_native_nv12_bo(device: *mut GbmDevice, modifier: u64) -> Result<*mut GbmBo> {
        let flags = if modifier == DRM_FORMAT_MOD_LINEAR {
            GBM_BO_USE_LINEAR
        } else {
            GBM_BO_USE_RENDERING
        };
        let mut bo = unsafe {
            gbm_bo_create_with_modifiers2(
                device,
                WIDTH,
                HEIGHT,
                DRM_FORMAT_NV12,
                &modifier,
                1,
                flags,
            )
        };
        if bo.is_null() && modifier == DRM_FORMAT_MOD_LINEAR {
            bo =
                unsafe { gbm_bo_create(device, WIDTH, HEIGHT, DRM_FORMAT_NV12, GBM_BO_USE_LINEAR) };
        }
        if bo.is_null() {
            bail!(
                "unsupported GBM NV12 modifier {modifier:#018x}; use a decoder-exported DMA-BUF for modifiers the allocator cannot create"
            );
        }
        Ok(bo)
    }

    fn paint_native_nv12(
        fd: &OwnedFd,
        y_offset: u64,
        y_stride: u32,
        uv_offset: u64,
        uv_stride: u32,
    ) -> Result<()> {
        let y_end = y_offset
            .checked_add(u64::from(y_stride) * u64::from(HEIGHT))
            .context("native NV12 Y layout overflow")?;
        let uv_height = HEIGHT.div_ceil(2);
        let uv_end = uv_offset
            .checked_add(u64::from(uv_stride) * u64::from(uv_height))
            .context("native NV12 UV layout overflow")?;
        let len = usize::try_from(y_end.max(uv_end))?;
        dma_buf_sync(fd, DMA_BUF_SYNC_WRITE)?;
        let mapping = unsafe {
            libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if mapping == libc::MAP_FAILED {
            let error = std::io::Error::last_os_error();
            let _ = dma_buf_sync(fd, DMA_BUF_SYNC_WRITE | DMA_BUF_SYNC_END);
            return Err(error).context("mmap failed for native NV12 DMA-BUF");
        }
        let bytes = unsafe { std::slice::from_raw_parts_mut(mapping.cast::<u8>(), len) };
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let offset = y_offset as usize + y as usize * y_stride as usize + x as usize;
                bytes[offset] = 16 + ((x + y) * 219 / (WIDTH + HEIGHT - 2)) as u8;
            }
        }
        for y in 0..uv_height {
            for x in 0..WIDTH.div_ceil(2) {
                let offset = uv_offset as usize + y as usize * uv_stride as usize + x as usize * 2;
                bytes[offset] = (16 + x * 224 / WIDTH.div_ceil(2).max(1)) as u8;
                bytes[offset + 1] = (240 - y * 224 / uv_height.max(1)) as u8;
            }
        }
        if let Err(error) = dma_buf_sync(fd, DMA_BUF_SYNC_WRITE | DMA_BUF_SYNC_END) {
            unsafe { libc::munmap(mapping, len) };
            return Err(error);
        }
        if unsafe { libc::munmap(mapping, len) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("munmap failed for native NV12 DMA-BUF");
        }
        Ok(())
    }

    fn dma_buf_sync(fd: &OwnedFd, flags: u64) -> Result<()> {
        let sync = DmaBufSync { flags };
        if unsafe { libc::ioctl(fd.as_raw_fd(), DMA_BUF_IOCTL_SYNC, &sync) } != 0 {
            return Err(std::io::Error::last_os_error()).context("DMA_BUF_IOCTL_SYNC failed");
        }
        Ok(())
    }

    fn dma_buf_identity(fd: &OwnedFd) -> Result<(u64, u64)> {
        let file = File::from(fd.try_clone()?);
        let metadata = file.metadata()?;
        use std::os::unix::fs::MetadataExt as _;
        Ok((metadata.dev(), metadata.ino()))
    }

    fn allocate_linear_bo(
        device: *mut GbmDevice,
        width: u32,
        height: u32,
        format: u32,
    ) -> Result<*mut GbmBo> {
        let modifier = DRM_FORMAT_MOD_LINEAR;
        let mut bo = unsafe {
            gbm_bo_create_with_modifiers2(
                device,
                width,
                height,
                format,
                &modifier,
                1,
                GBM_BO_USE_LINEAR,
            )
        };
        if bo.is_null()
            && unsafe { gbm_device_is_format_supported(device, format, GBM_BO_USE_LINEAR) != 0 }
        {
            bo = unsafe { gbm_bo_create(device, width, height, format, GBM_BO_USE_LINEAR) };
        }
        if bo.is_null() {
            bail!("unsupported GBM format {format:#010x}");
        }
        Ok(bo)
    }

    fn export_linear_plane(bo: *mut GbmBo) -> Result<DmaBufPlane> {
        let plane_count = unsafe { gbm_bo_get_plane_count(bo) };
        if plane_count != 1 {
            bail!("expected one GBM plane, got {plane_count}");
        }
        let raw_fd = unsafe { gbm_bo_get_fd_for_plane(bo, 0) };
        if raw_fd < 0 {
            bail!("GBM failed to export a plane as a DMA-BUF fd");
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        let stride = unsafe { gbm_bo_get_stride_for_plane(bo, 0) };
        let offset = u64::from(unsafe { gbm_bo_get_offset(bo, 0) });
        let modifier = match unsafe { gbm_bo_get_modifier(bo) } {
            DRM_FORMAT_MOD_INVALID => DRM_FORMAT_MOD_LINEAR,
            modifier => modifier,
        };
        Ok(DmaBufPlane::new(fd, modifier, offset, stride))
    }

    fn paint_nv12_y_plane(bo: *mut GbmBo) -> Result<()> {
        paint_plane(bo, WIDTH, HEIGHT, 1, |x, y, pixel| {
            let value = 16 + ((x + y) * 219 / (WIDTH + HEIGHT - 2)) as u8;
            unsafe { pixel.write(value) };
        })
    }

    fn paint_nv12_uv_plane(bo: *mut GbmBo) -> Result<()> {
        let width = WIDTH.div_ceil(2);
        let height = HEIGHT.div_ceil(2);
        paint_plane(bo, width, height, 2, |x, y, pixel| unsafe {
            pixel.write((16 + x * 224 / width.max(1)) as u8);
            pixel.add(1).write((240 - y * 224 / height.max(1)) as u8);
        })
    }

    fn paint_plane(
        bo: *mut GbmBo,
        width: u32,
        height: u32,
        bytes_per_pixel: usize,
        mut paint: impl FnMut(u32, u32, *mut u8),
    ) -> Result<()> {
        let mut mapped_stride = 0;
        let mut map_data = ptr::null_mut();
        let pixels = unsafe {
            gbm_bo_map(
                bo,
                0,
                0,
                width,
                height,
                GBM_BO_TRANSFER_WRITE,
                &mut mapped_stride,
                &mut map_data,
            )
        };
        if pixels.is_null() {
            bail!("gbm_bo_map failed");
        }
        for y in 0..height {
            for x in 0..width {
                let pixel = unsafe {
                    (pixels as *mut u8)
                        .add(y as usize * mapped_stride as usize + x as usize * bytes_per_pixel)
                };
                paint(x, y, pixel);
            }
        }
        unsafe { gbm_bo_unmap(bo, map_data) };
        Ok(())
    }

    fn paint_test_pattern(bo: *mut GbmBo) -> Result<()> {
        let mut mapped_stride = 0;
        let mut map_data = ptr::null_mut();
        let pixels = unsafe {
            gbm_bo_map(
                bo,
                0,
                0,
                WIDTH,
                HEIGHT,
                GBM_BO_TRANSFER_WRITE,
                &mut mapped_stride,
                &mut map_data,
            )
        };
        if pixels.is_null() {
            bail!("gbm_bo_map failed");
        }

        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                let phase = x as f32 / WIDTH as f32;
                let wave = y as f32 / HEIGHT as f32;
                let offset = y * mapped_stride as usize + x * 4;
                let pixel = unsafe { (pixels as *mut u8).add(offset) };
                unsafe {
                    pixel.write((255.0 * (1.0 - phase)) as u8);
                    pixel.add(1).write((255.0 * wave) as u8);
                    pixel.add(2).write((255.0 * phase) as u8);
                    pixel.add(3).write(255);
                }
            }
        }

        unsafe { gbm_bo_unmap(bo, map_data) };
        Ok(())
    }

    fn find_render_node(specs: &GpuSpecs) -> Result<PathBuf> {
        if let Ok(path) = std::env::var("GPUI_DMA_BUF_RENDER_NODE") {
            return Ok(path.into());
        }

        let expected_vendor = gpu_vendor_id(&specs.device_name);
        let mut candidates = fs::read_dir("/dev/dri")?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("renderD"))
            })
            .collect::<Vec<_>>();
        candidates.sort();

        if let Some(expected_vendor) = expected_vendor
            && let Some(path) = candidates
                .iter()
                .find(|path| drm_vendor_id(path).is_some_and(|vendor| vendor == expected_vendor))
        {
            return Ok(path.clone());
        }

        candidates
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no DRM render node found; set GPUI_DMA_BUF_RENDER_NODE"))
    }

    fn gpu_vendor_id(device_name: &str) -> Option<u16> {
        let name = device_name.to_ascii_lowercase();
        if name.contains("nvidia") {
            Some(0x10de)
        } else if name.contains("amd") || name.contains("radeon") {
            Some(0x1002)
        } else if name.contains("intel") {
            Some(0x8086)
        } else {
            None
        }
    }

    fn drm_vendor_id(render_node: &Path) -> Option<u16> {
        let name = render_node.file_name()?;
        let vendor =
            fs::read_to_string(Path::new("/sys/class/drm").join(name).join("device/vendor"))
                .ok()?;
        u16::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok()
    }

    pub fn run() {
        application().run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(1100.0), px(700.0)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| DmaBufExample {
                        handle: SurfaceHandle::new(),
                        imported: None,
                        error: None,
                        attempted: false,
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
    }
}

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("dma_buf_surface is available only on Linux");
}
