#![cfg(not(target_family = "wasm"))]

use std::sync::Barrier;

use gpui_wgpu::WgpuContext;

#[test]
#[ignore = "requires a GPU adapter"]
fn concurrent_context_creation_and_submission() -> anyhow::Result<()> {
    let start = Barrier::new(4);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let start = &start;
                scope.spawn(move || -> anyhow::Result<()> {
                    start.wait();
                    for _ in 0..8 {
                        let context = WgpuContext::new_headless()?;
                        let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("context initialization buffer"),
                            size: 4,
                            usage: wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        context.queue.write_buffer(&buffer, 0, &[1, 2, 3, 4]);
                        context.queue.submit([]);
                        context.device.poll(wgpu::PollType::wait_indefinitely())?;
                    }
                    Ok(())
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("GPU initialization worker panicked")?;
        }
        Ok(())
    })
}
