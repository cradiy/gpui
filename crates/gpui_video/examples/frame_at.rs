use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui_video::{MediaSource, VideoFrameExtractor};

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let input = arguments
        .next()
        .context("usage: cargo run -p gpui_video --example frame_at -- <media> [seconds]")?;
    let seconds = arguments
        .next()
        .map(|value| value.parse::<f64>())
        .transpose()
        .context("frame timestamp must be a number of seconds")?
        .unwrap_or_default();
    if !seconds.is_finite() || seconds < 0.0 {
        anyhow::bail!("frame timestamp must be finite and non-negative");
    }

    let extractor = VideoFrameExtractor::new(MediaSource::parse(input)?)?;
    let frame = extractor.frame_at_blocking(Duration::from_secs_f64(seconds))?;
    let size = frame.coded_size();

    println!("timestamp={:?}", frame.timestamp());
    println!("duration={:?}", frame.duration());
    println!("size={}x{}", size.width.0, size.height.0);
    println!("transport={:?}", frame.transport());
    Ok(())
}
