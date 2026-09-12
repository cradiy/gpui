use std::time::Duration;

use gpui_media_system::{MediaSource, VideoFrameExtractor};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let input = arguments.next().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: cargo run -p gpui_media_system --example frame_at -- <media> [seconds]",
        )
    })?;
    let seconds = arguments
        .next()
        .map(|value| value.parse::<f64>())
        .transpose()
        .map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("frame timestamp must be a number of seconds: {error}"),
            )
        })?
        .unwrap_or_default();
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "frame timestamp must be finite and non-negative",
        )
        .into());
    }

    let extractor = VideoFrameExtractor::new(
        MediaSource::parse(input)?,
        std::sync::Arc::new(gpui_media_system::SystemBackend),
    )?;
    let frame = extractor.frame_at_blocking(Duration::from_secs_f64(seconds))?;
    let size = frame.coded_size();

    println!("timestamp={:?}", frame.timestamp());
    println!("duration={:?}", frame.duration());
    println!("size={}x{}", size.width, size.height);
    println!("transport={:?}", frame.transport());
    Ok(())
}
