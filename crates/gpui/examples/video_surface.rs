#![cfg_attr(target_family = "wasm", no_main)]

use std::time::Instant;

use gpui::{
    App, Bounds, ColorRange, Context, SurfaceColorInfo, SurfaceFrame, SurfaceHandle, Window,
    WindowBounds, WindowOptions, YuvMatrix, div, prelude::*, px, size, surface,
};
use gpui_platform::application;

const FRAME_WIDTH: usize = 480;
const FRAME_HEIGHT: usize = 270;

struct VideoSurfaceExample {
    started_at: Instant,
    bgra_handle: SurfaceHandle,
    nv12_handle: SurfaceHandle,
}

impl VideoSurfaceExample {
    fn bgra_frame(&self, sequence: u64) -> SurfaceFrame {
        let mut bytes = vec![0; FRAME_WIDTH * FRAME_HEIGHT * 4];
        let shift = (sequence as usize * 3) % FRAME_WIDTH;
        for y in 0..FRAME_HEIGHT {
            for x in 0..FRAME_WIDTH {
                let offset = (y * FRAME_WIDTH + x) * 4;
                let phase = ((x + shift) % FRAME_WIDTH) as f32 / FRAME_WIDTH as f32;
                bytes[offset] = (255.0 * phase) as u8;
                bytes[offset + 1] = (255.0 * y as f32 / FRAME_HEIGHT as f32) as u8;
                bytes[offset + 2] = (255.0 * (1.0 - phase)) as u8;
                bytes[offset + 3] = 255;
            }
        }

        SurfaceFrame::bgra(
            self.bgra_handle.clone(),
            sequence,
            size(FRAME_WIDTH.into(), FRAME_HEIGHT.into()),
            bytes,
            (FRAME_WIDTH * 4) as u32,
        )
        .unwrap()
    }

    fn nv12_frame(&self, sequence: u64) -> SurfaceFrame {
        let mut y_plane = vec![0; FRAME_WIDTH * FRAME_HEIGHT];
        let mut uv_plane = vec![0; FRAME_WIDTH * FRAME_HEIGHT / 2];
        let shift = (sequence as usize * 2) % FRAME_WIDTH;

        for y in 0..FRAME_HEIGHT {
            for x in 0..FRAME_WIDTH {
                let wave = ((x + shift) % FRAME_WIDTH) as f32 / FRAME_WIDTH as f32;
                y_plane[y * FRAME_WIDTH + x] = (32.0 + wave * 190.0) as u8;
            }
        }
        for y in 0..FRAME_HEIGHT / 2 {
            for x in 0..FRAME_WIDTH / 2 {
                let offset = y * FRAME_WIDTH + x * 2;
                let phase = ((x * 2 + shift) % FRAME_WIDTH) as f32 / FRAME_WIDTH as f32;
                uv_plane[offset] = (64.0 + phase * 128.0) as u8;
                uv_plane[offset + 1] = (192.0 - phase * 128.0) as u8;
            }
        }

        SurfaceFrame::nv12(
            self.nv12_handle.clone(),
            sequence,
            size(FRAME_WIDTH.into(), FRAME_HEIGHT.into()),
            y_plane,
            FRAME_WIDTH as u32,
            uv_plane,
            FRAME_WIDTH as u32,
            SurfaceColorInfo {
                matrix: YuvMatrix::Bt709,
                range: ColorRange::Limited,
            },
        )
        .unwrap()
    }
}

impl Render for VideoSurfaceExample {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();
        let sequence = self.started_at.elapsed().as_millis() as u64 / 16;
        let dma_buf_status = if window
            .gpu_specs()
            .is_some_and(|specs| specs.supports_dma_buf_import)
        {
            "Linux DMA-BUF import: available"
        } else {
            "Linux DMA-BUF import: unavailable"
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .p_6()
            .bg(gpui::rgb(0x11151b))
            .text_color(gpui::white())
            .child("Dynamic video Surface: stable handles, changing sequences")
            .child(dma_buf_status)
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child("BGRA8")
                            .child(
                                surface(self.bgra_frame(sequence))
                                    .w_full()
                                    .h(px(270.0))
                                    .rounded_xl()
                                    .overflow_hidden(),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child("NV12 · BT.709 limited")
                            .child(
                                surface(self.nv12_frame(sequence))
                                    .w_full()
                                    .h(px(270.0))
                                    .rounded_xl()
                                    .overflow_hidden(),
                            ),
                    ),
            )
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(420.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| VideoSurfaceExample {
                    started_at: Instant::now(),
                    bgra_handle: SurfaceHandle::new(),
                    nv12_handle: SurfaceHandle::new(),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
