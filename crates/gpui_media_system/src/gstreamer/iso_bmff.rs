//! ISO BMFF helpers used by the GStreamer system backend.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    time::Duration,
};

use gpui_media::MediaSource;

const BOX_HEADER_SIZE: u64 = 8;

pub(super) fn fragmented_duration(source: &MediaSource) -> Option<Duration> {
    let path = local_file_path(source)?;
    let mut file = File::open(path).ok()?;
    match probe_fragmented_duration(&mut file) {
        Ok(duration) => duration,
        Err(error) => {
            log::debug!("failed to probe fragmented MP4 duration: {error}");
            None
        }
    }
}

fn local_file_path(source: &MediaSource) -> Option<PathBuf> {
    let uri = url::Url::parse(source.uri()).ok()?;
    (uri.scheme() == "file")
        .then(|| uri.to_file_path().ok())
        .flatten()
}

fn probe_fragmented_duration(reader: &mut (impl Read + Seek)) -> io::Result<Option<Duration>> {
    let file_end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;

    let mut movie = MovieMetadata::default();
    let mut fragments = Vec::new();
    while let Some(header) = read_box_header(reader, file_end)? {
        match &header.kind {
            b"moov" => parse_movie(reader, header.content_start, header.end, &mut movie)?,
            b"moof" => fragments.push((header.content_start, header.end)),
            _ => {}
        }
        reader.seek(SeekFrom::Start(header.end))?;
    }

    if fragments.is_empty() || movie.video_tracks.is_empty() {
        return Ok(None);
    }

    let mut next_decode_time = HashMap::<u32, u64>::new();
    let mut video_duration = HashMap::<u32, u64>::new();
    for (start, end) in fragments {
        for fragment in parse_movie_fragment(reader, start, end, &movie.default_durations)? {
            let start = fragment
                .base_decode_time
                .or_else(|| next_decode_time.get(&fragment.track_id).copied())
                .unwrap_or_default();
            let Some(end) = start.checked_add(fragment.duration) else {
                continue;
            };
            next_decode_time.insert(fragment.track_id, end);
            if movie.video_tracks.contains(&fragment.track_id) {
                video_duration
                    .entry(fragment.track_id)
                    .and_modify(|duration| *duration = (*duration).max(end))
                    .or_insert(end);
            }
        }
    }

    let duration = video_duration
        .into_iter()
        .filter_map(|(track_id, value)| {
            let timescale = movie.timescales.get(&track_id).copied()?;
            duration_from_timescale(value, timescale)
        })
        .max();
    Ok(duration.filter(|duration| !duration.is_zero()))
}

#[derive(Default)]
struct MovieMetadata {
    video_tracks: HashSet<u32>,
    timescales: HashMap<u32, u32>,
    default_durations: HashMap<u32, u32>,
}

fn parse_movie(
    reader: &mut (impl Read + Seek),
    start: u64,
    end: u64,
    movie: &mut MovieMetadata,
) -> io::Result<()> {
    reader.seek(SeekFrom::Start(start))?;
    while let Some(header) = read_box_header(reader, end)? {
        match &header.kind {
            b"trak" => {
                if let Some(track) = parse_track(reader, header.content_start, header.end)? {
                    movie.timescales.insert(track.id, track.timescale);
                    if track.is_video {
                        movie.video_tracks.insert(track.id);
                    }
                }
            }
            b"mvex" => parse_movie_extends(
                reader,
                header.content_start,
                header.end,
                &mut movie.default_durations,
            )?,
            _ => {}
        }
        reader.seek(SeekFrom::Start(header.end))?;
    }
    Ok(())
}

struct TrackMetadata {
    id: u32,
    timescale: u32,
    is_video: bool,
}

fn parse_track(
    reader: &mut (impl Read + Seek),
    start: u64,
    end: u64,
) -> io::Result<Option<TrackMetadata>> {
    let mut track_id = None;
    let mut timescale = None;
    let mut is_video = false;

    reader.seek(SeekFrom::Start(start))?;
    while let Some(header) = read_box_header(reader, end)? {
        match &header.kind {
            b"tkhd" => track_id = parse_track_id(reader, &header)?,
            b"mdia" => {
                let media = parse_media(reader, header.content_start, header.end)?;
                timescale = media.timescale;
                is_video = media.handler == *b"vide";
            }
            _ => {}
        }
        reader.seek(SeekFrom::Start(header.end))?;
    }

    Ok(track_id
        .zip(timescale)
        .filter(|(_, timescale)| *timescale != 0)
        .map(|(id, timescale)| TrackMetadata {
            id,
            timescale,
            is_video,
        }))
}

fn parse_track_id(reader: &mut (impl Read + Seek), header: &BoxHeader) -> io::Result<Option<u32>> {
    let Some(version) = read_u8_at(reader, header.content_start, header.end)? else {
        return Ok(None);
    };
    let offset = match version {
        0 => 12,
        1 => 20,
        _ => return Ok(None),
    };
    read_u32_at(reader, header.content_start + offset, header.end)
}

#[derive(Default)]
struct MediaMetadata {
    timescale: Option<u32>,
    handler: [u8; 4],
}

fn parse_media(reader: &mut (impl Read + Seek), start: u64, end: u64) -> io::Result<MediaMetadata> {
    let mut media = MediaMetadata::default();
    reader.seek(SeekFrom::Start(start))?;
    while let Some(header) = read_box_header(reader, end)? {
        match &header.kind {
            b"mdhd" => media.timescale = parse_media_timescale(reader, &header)?,
            b"hdlr" => {
                if let Some(handler) = read_fourcc_at(reader, header.content_start + 8, header.end)?
                {
                    media.handler = handler;
                }
            }
            _ => {}
        }
        reader.seek(SeekFrom::Start(header.end))?;
    }
    Ok(media)
}

fn parse_media_timescale(
    reader: &mut (impl Read + Seek),
    header: &BoxHeader,
) -> io::Result<Option<u32>> {
    let Some(version) = read_u8_at(reader, header.content_start, header.end)? else {
        return Ok(None);
    };
    let offset = match version {
        0 => 12,
        1 => 20,
        _ => return Ok(None),
    };
    read_u32_at(reader, header.content_start + offset, header.end)
}

fn parse_movie_extends(
    reader: &mut (impl Read + Seek),
    start: u64,
    end: u64,
    default_durations: &mut HashMap<u32, u32>,
) -> io::Result<()> {
    reader.seek(SeekFrom::Start(start))?;
    while let Some(header) = read_box_header(reader, end)? {
        if header.kind == *b"trex"
            && let (Some(track_id), Some(default_duration)) = (
                read_u32_at(reader, header.content_start + 4, header.end)?,
                read_u32_at(reader, header.content_start + 12, header.end)?,
            )
        {
            default_durations.insert(track_id, default_duration);
        }
        reader.seek(SeekFrom::Start(header.end))?;
    }
    Ok(())
}

struct TrackFragment {
    track_id: u32,
    base_decode_time: Option<u64>,
    duration: u64,
}

fn parse_movie_fragment(
    reader: &mut (impl Read + Seek),
    start: u64,
    end: u64,
    default_durations: &HashMap<u32, u32>,
) -> io::Result<Vec<TrackFragment>> {
    let mut fragments = Vec::new();
    reader.seek(SeekFrom::Start(start))?;
    while let Some(header) = read_box_header(reader, end)? {
        if header.kind == *b"traf"
            && let Some(fragment) =
                parse_track_fragment(reader, header.content_start, header.end, default_durations)?
        {
            fragments.push(fragment);
        }
        reader.seek(SeekFrom::Start(header.end))?;
    }
    Ok(fragments)
}

fn parse_track_fragment(
    reader: &mut (impl Read + Seek),
    start: u64,
    end: u64,
    default_durations: &HashMap<u32, u32>,
) -> io::Result<Option<TrackFragment>> {
    let mut track_id = None;
    let mut base_decode_time = None;
    let mut tfhd_default_duration = None;
    let mut runs = Vec::new();

    reader.seek(SeekFrom::Start(start))?;
    while let Some(header) = read_box_header(reader, end)? {
        match &header.kind {
            b"tfhd" => {
                let tfhd = parse_track_fragment_header(reader, &header)?;
                track_id = tfhd.track_id;
                tfhd_default_duration = tfhd.default_sample_duration;
            }
            b"tfdt" => base_decode_time = parse_base_decode_time(reader, &header)?,
            b"trun" => runs.push(parse_track_run(reader, &header)?),
            _ => {}
        }
        reader.seek(SeekFrom::Start(header.end))?;
    }

    let Some(track_id) = track_id else {
        return Ok(None);
    };
    let default_duration = tfhd_default_duration
        .or_else(|| default_durations.get(&track_id).copied())
        .unwrap_or_default();
    let mut duration = 0_u64;
    for run in runs {
        let run_duration = match run.sample_durations {
            Some(sample_durations) => sample_durations.into_iter().map(u64::from).sum(),
            None => u64::from(default_duration).saturating_mul(u64::from(run.sample_count)),
        };
        duration = duration.saturating_add(run_duration);
    }

    Ok((duration != 0).then_some(TrackFragment {
        track_id,
        base_decode_time,
        duration,
    }))
}

#[derive(Default)]
struct TrackFragmentHeader {
    track_id: Option<u32>,
    default_sample_duration: Option<u32>,
}

fn parse_track_fragment_header(
    reader: &mut (impl Read + Seek),
    header: &BoxHeader,
) -> io::Result<TrackFragmentHeader> {
    let Some(flags) = read_full_box_flags(reader, header)? else {
        return Ok(TrackFragmentHeader::default());
    };
    let track_id = read_u32_at(reader, header.content_start + 4, header.end)?;
    let mut offset = header.content_start + 8;
    if flags & 0x000001 != 0 {
        offset = offset.saturating_add(8);
    }
    if flags & 0x000002 != 0 {
        offset = offset.saturating_add(4);
    }
    let default_sample_duration = if flags & 0x000008 != 0 {
        read_u32_at(reader, offset, header.end)?
    } else {
        None
    };

    Ok(TrackFragmentHeader {
        track_id,
        default_sample_duration,
    })
}

fn parse_base_decode_time(
    reader: &mut (impl Read + Seek),
    header: &BoxHeader,
) -> io::Result<Option<u64>> {
    let Some(version) = read_u8_at(reader, header.content_start, header.end)? else {
        return Ok(None);
    };
    match version {
        0 => Ok(read_u32_at(reader, header.content_start + 4, header.end)?.map(u64::from)),
        1 => read_u64_at(reader, header.content_start + 4, header.end),
        _ => Ok(None),
    }
}

struct TrackRun {
    sample_count: u32,
    sample_durations: Option<Vec<u32>>,
}

fn parse_track_run(reader: &mut (impl Read + Seek), header: &BoxHeader) -> io::Result<TrackRun> {
    let flags = read_full_box_flags(reader, header)?.unwrap_or_default();
    let sample_count =
        read_u32_at(reader, header.content_start + 4, header.end)?.unwrap_or_default();
    let mut offset = header.content_start + 8;
    if flags & 0x000001 != 0 {
        offset = offset.saturating_add(4);
    }
    if flags & 0x000004 != 0 {
        offset = offset.saturating_add(4);
    }

    let has_duration = flags & 0x000100 != 0;
    let sample_size = u64::from((flags & 0x000200 != 0) as u8) * 4;
    let sample_flags = u64::from((flags & 0x000400 != 0) as u8) * 4;
    let composition_offset = u64::from((flags & 0x000800 != 0) as u8) * 4;
    let entry_size =
        u64::from(has_duration as u8) * 4 + sample_size + sample_flags + composition_offset;
    let required = entry_size.saturating_mul(u64::from(sample_count));
    if offset.saturating_add(required) > header.end {
        return Ok(TrackRun {
            sample_count: 0,
            sample_durations: None,
        });
    }

    let sample_durations = if has_duration {
        let mut durations = Vec::with_capacity(sample_count as usize);
        for _ in 0..sample_count {
            let Some(duration) = read_u32_at(reader, offset, header.end)? else {
                return Ok(TrackRun {
                    sample_count: 0,
                    sample_durations: None,
                });
            };
            durations.push(duration);
            offset += entry_size;
        }
        Some(durations)
    } else {
        None
    };

    Ok(TrackRun {
        sample_count,
        sample_durations,
    })
}

#[derive(Clone, Copy)]
struct BoxHeader {
    kind: [u8; 4],
    content_start: u64,
    end: u64,
}

fn read_box_header(
    reader: &mut (impl Read + Seek),
    parent_end: u64,
) -> io::Result<Option<BoxHeader>> {
    let start = reader.stream_position()?;
    if start == parent_end {
        return Ok(None);
    }
    if start > parent_end || parent_end - start < BOX_HEADER_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "truncated ISO BMFF box header",
        ));
    }

    let mut header = [0_u8; 8];
    reader.read_exact(&mut header)?;
    let size32 = u32::from_be_bytes(header[..4].try_into().expect("four bytes"));
    let kind = header[4..8].try_into().expect("four bytes");
    let (size, header_size) = match size32 {
        0 => (parent_end - start, BOX_HEADER_SIZE),
        1 => {
            let mut extended = [0_u8; 8];
            reader.read_exact(&mut extended)?;
            (u64::from_be_bytes(extended), 16)
        }
        size => (u64::from(size), BOX_HEADER_SIZE),
    };
    if size < header_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid ISO BMFF box size",
        ));
    }
    let end = start
        .checked_add(size)
        .filter(|end| *end <= parent_end)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "ISO BMFF box exceeds parent"))?;

    Ok(Some(BoxHeader {
        kind,
        content_start: start + header_size,
        end,
    }))
}

fn read_full_box_flags(
    reader: &mut (impl Read + Seek),
    header: &BoxHeader,
) -> io::Result<Option<u32>> {
    let Some(bytes) = read_array_at::<4>(reader, header.content_start, header.end)? else {
        return Ok(None);
    };
    Ok(Some(
        (u32::from(bytes[1]) << 16) | (u32::from(bytes[2]) << 8) | u32::from(bytes[3]),
    ))
}

fn read_u8_at(reader: &mut (impl Read + Seek), offset: u64, end: u64) -> io::Result<Option<u8>> {
    Ok(read_array_at::<1>(reader, offset, end)?.map(|bytes| bytes[0]))
}

fn read_u32_at(reader: &mut (impl Read + Seek), offset: u64, end: u64) -> io::Result<Option<u32>> {
    Ok(read_array_at::<4>(reader, offset, end)?.map(u32::from_be_bytes))
}

fn read_u64_at(reader: &mut (impl Read + Seek), offset: u64, end: u64) -> io::Result<Option<u64>> {
    Ok(read_array_at::<8>(reader, offset, end)?.map(u64::from_be_bytes))
}

fn read_fourcc_at(
    reader: &mut (impl Read + Seek),
    offset: u64,
    end: u64,
) -> io::Result<Option<[u8; 4]>> {
    read_array_at(reader, offset, end)
}

fn read_array_at<const N: usize>(
    reader: &mut (impl Read + Seek),
    offset: u64,
    end: u64,
) -> io::Result<Option<[u8; N]>> {
    let Some(read_end) = offset.checked_add(N as u64) else {
        return Ok(None);
    };
    if read_end > end {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = [0_u8; N];
    reader.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

fn duration_from_timescale(value: u64, timescale: u32) -> Option<Duration> {
    let seconds = value / u64::from(timescale);
    let remainder = value % u64::from(timescale);
    let nanos = u128::from(remainder)
        .checked_mul(1_000_000_000)?
        .checked_div(u128::from(timescale))?;
    Some(Duration::new(seconds, u32::try_from(nanos).ok()?))
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, time::Duration};

    use super::probe_fragmented_duration;

    #[test]
    fn derives_duration_from_video_fragments() {
        let movie = mp4_box(
            b"moov",
            [
                track(1, 1_000, *b"vide"),
                track(2, 48_000, *b"soun"),
                mp4_box(
                    b"mvex",
                    [track_extends(1, 20), track_extends(2, 1_024)].concat(),
                ),
            ]
            .concat(),
        );
        let first_fragment = movie_fragment([
            track_fragment(1, 0, 250, None),
            track_fragment(2, 0, 300, None),
        ]);
        let second_fragment = movie_fragment([
            track_fragment(1, 5_000, 3, Some(&[1_000, 1_000, 1_000])),
            track_fragment(2, 307_200, 400, None),
        ]);
        let bytes = [
            mp4_box(b"ftyp", b"iso5".to_vec()),
            movie,
            first_fragment,
            mp4_box(b"mdat", Vec::new()),
            second_fragment,
            mp4_box(b"mdat", Vec::new()),
        ]
        .concat();

        assert_eq!(
            probe_fragmented_duration(&mut Cursor::new(bytes)).unwrap(),
            Some(Duration::from_secs(8))
        );
    }

    #[test]
    fn ignores_non_fragmented_movies() {
        let bytes = mp4_box(b"moov", track(1, 1_000, *b"vide"));
        assert_eq!(
            probe_fragmented_duration(&mut Cursor::new(bytes)).unwrap(),
            None
        );
    }

    fn track(track_id: u32, timescale: u32, handler: [u8; 4]) -> Vec<u8> {
        let mut track_header = vec![0; 24];
        track_header[12..16].copy_from_slice(&track_id.to_be_bytes());
        let mut media_header = vec![0; 20];
        media_header[12..16].copy_from_slice(&timescale.to_be_bytes());
        let mut handler_box = vec![0; 12];
        handler_box[8..12].copy_from_slice(&handler);
        mp4_box(
            b"trak",
            [
                mp4_box(b"tkhd", track_header),
                mp4_box(
                    b"mdia",
                    [
                        mp4_box(b"mdhd", media_header),
                        mp4_box(b"hdlr", handler_box),
                    ]
                    .concat(),
                ),
            ]
            .concat(),
        )
    }

    fn track_extends(track_id: u32, default_duration: u32) -> Vec<u8> {
        let mut body = vec![0; 24];
        body[4..8].copy_from_slice(&track_id.to_be_bytes());
        body[12..16].copy_from_slice(&default_duration.to_be_bytes());
        mp4_box(b"trex", body)
    }

    fn movie_fragment<const N: usize>(tracks: [Vec<u8>; N]) -> Vec<u8> {
        mp4_box(b"moof", tracks.concat())
    }

    fn track_fragment(
        track_id: u32,
        base_decode_time: u64,
        sample_count: u32,
        sample_durations: Option<&[u32]>,
    ) -> Vec<u8> {
        let mut fragment_header = vec![0; 8];
        fragment_header[4..8].copy_from_slice(&track_id.to_be_bytes());

        let mut decode_time = vec![1, 0, 0, 0];
        decode_time.extend_from_slice(&base_decode_time.to_be_bytes());

        let mut run = vec![0, 0, 0, 0];
        if sample_durations.is_some() {
            run[2] = 1;
        }
        run.extend_from_slice(&sample_count.to_be_bytes());
        for duration in sample_durations.into_iter().flatten() {
            run.extend_from_slice(&duration.to_be_bytes());
        }

        mp4_box(
            b"traf",
            [
                mp4_box(b"tfhd", fragment_header),
                mp4_box(b"tfdt", decode_time),
                mp4_box(b"trun", run),
            ]
            .concat(),
        )
    }

    fn mp4_box(kind: &[u8; 4], body: Vec<u8>) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(body.len() + 8);
        bytes.extend_from_slice(&u32::try_from(body.len() + 8).unwrap().to_be_bytes());
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(&body);
        bytes
    }
}
