use std::{sync::Arc, time::Duration};

use crate::{MediaError, MediaResult, MediaStreamId};

/// A supported text-subtitle format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubtitleFormat {
    SubRip,
    WebVtt,
    Ass,
    /// Text already decoded by a playback backend.
    PlainText,
}

/// One normalized subtitle cue.
///
/// `text` is suitable for basic host rendering. `raw` and `settings` retain
/// format-specific information for hosts that need richer presentation.
#[derive(Clone, Debug)]
pub struct SubtitleCue {
    pub id: Option<Arc<str>>,
    pub start: Duration,
    pub end: Duration,
    pub text: Arc<str>,
    pub raw: Arc<str>,
    pub settings: Option<Arc<str>>,
    pub format: SubtitleFormat,
}

/// A parsed external subtitle document.
#[derive(Clone, Debug)]
pub struct ParsedSubtitles {
    pub format: SubtitleFormat,
    pub cues: Arc<[SubtitleCue]>,
}

/// Incremental subtitle output produced by an active playback backend.
///
/// Hosts own cue storage, active-cue calculation, composition and rendering.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SubtitleEvent {
    /// A seek, stream switch, reload or stop invalidated previously emitted cues.
    Reset,
    /// A newly extracted cue from the selected embedded subtitle stream.
    Cue {
        stream_id: MediaStreamId,
        cue: Arc<SubtitleCue>,
    },
}

/// Parses subtitle source text into normalized, time-ordered cues.
///
/// File access, network loading and character-set decoding are intentionally
/// owned by the host. Pass `None` to detect SRT, WebVTT or ASS/SSA from content.
pub fn parse_subtitles(
    source: &str,
    format: Option<SubtitleFormat>,
) -> MediaResult<ParsedSubtitles> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let format = format
        .or_else(|| detect_subtitle_format(source))
        .ok_or_else(|| MediaError::invalid_input("could not detect the subtitle format"))?;
    let mut cues = match format {
        SubtitleFormat::SubRip => parse_subrip(source)?,
        SubtitleFormat::WebVtt => parse_webvtt(source)?,
        SubtitleFormat::Ass => parse_ass(source)?,
        SubtitleFormat::PlainText => {
            return Err(MediaError::invalid_input(
                "plain subtitle samples require backend timestamps",
            ));
        }
    };
    cues.sort_by_key(|cue| (cue.start, cue.end));
    Ok(ParsedSubtitles {
        format,
        cues: cues.into(),
    })
}

pub fn detect_subtitle_format(source: &str) -> Option<SubtitleFormat> {
    let source = source.trim_start_matches('\u{feff}').trim_start();
    if source.starts_with("WEBVTT") {
        return Some(SubtitleFormat::WebVtt);
    }
    if source.lines().any(|line| {
        let line = line.trim();
        line.eq_ignore_ascii_case("[Script Info]")
            || line.eq_ignore_ascii_case("[Events]")
            || line.starts_with("Dialogue:")
    }) {
        return Some(SubtitleFormat::Ass);
    }
    source
        .lines()
        .any(|line| line.contains("-->") && line.contains(','))
        .then_some(SubtitleFormat::SubRip)
        .or_else(|| {
            source
                .lines()
                .any(|line| line.contains("-->"))
                .then_some(SubtitleFormat::WebVtt)
        })
}

fn parse_subrip(source: &str) -> MediaResult<Vec<SubtitleCue>> {
    let normalized = normalize_newlines(source);
    let mut cues = Vec::new();
    for block in split_blocks(&normalized) {
        let lines = block.lines().collect::<Vec<_>>();
        let Some(timing_index) = lines.iter().position(|line| line.contains("-->")) else {
            continue;
        };
        let (start, end, settings) = parse_timing_line(lines[timing_index], false)?;
        let payload = lines[timing_index + 1..].join("\n");
        if payload.trim().is_empty() {
            continue;
        }
        let id = lines[..timing_index]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty())
            .map(|id| id.to_owned().into());
        cues.push(SubtitleCue {
            id,
            start,
            end,
            text: normalize_markup_text(&payload).into(),
            raw: block.to_owned().into(),
            settings: settings.map(Into::into),
            format: SubtitleFormat::SubRip,
        });
    }
    require_cues(cues, SubtitleFormat::SubRip)
}

fn parse_webvtt(source: &str) -> MediaResult<Vec<SubtitleCue>> {
    let normalized = normalize_newlines(source);
    let body = normalized
        .strip_prefix("WEBVTT")
        .map(|body| {
            body.trim_start_matches(|character| character != '\n')
                .trim_start()
        })
        .unwrap_or(&normalized);
    let mut cues = Vec::new();
    for block in split_blocks(body) {
        let lines = block.lines().collect::<Vec<_>>();
        if lines.is_empty()
            || matches!(
                lines[0].trim(),
                line if line.starts_with("NOTE")
                    || line == "STYLE"
                    || line.starts_with("REGION")
            )
        {
            continue;
        }
        let Some(timing_index) = lines.iter().position(|line| line.contains("-->")) else {
            continue;
        };
        let (start, end, settings) = parse_timing_line(lines[timing_index], true)?;
        let payload = lines[timing_index + 1..].join("\n");
        if payload.trim().is_empty() {
            continue;
        }
        let id = lines[..timing_index]
            .iter()
            .map(|line| line.trim())
            .find(|line| !line.is_empty())
            .map(|id| id.to_owned().into());
        cues.push(SubtitleCue {
            id,
            start,
            end,
            text: normalize_markup_text(&payload).into(),
            raw: block.to_owned().into(),
            settings: settings.map(Into::into),
            format: SubtitleFormat::WebVtt,
        });
    }
    require_cues(cues, SubtitleFormat::WebVtt)
}

fn parse_ass(source: &str) -> MediaResult<Vec<SubtitleCue>> {
    let normalized = normalize_newlines(source);
    let mut in_events = false;
    let mut fields = default_ass_fields();
    let mut cues = Vec::new();
    for line in normalized.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_events = line.eq_ignore_ascii_case("[Events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(format) = strip_ascii_prefix(line, "Format:") {
            fields = format
                .split(',')
                .map(|field| field.trim().to_ascii_lowercase())
                .collect();
            continue;
        }
        let Some(dialogue) = strip_ascii_prefix(line, "Dialogue:") else {
            continue;
        };
        let parts = dialogue
            .splitn(fields.len().max(1), ',')
            .map(str::trim)
            .collect::<Vec<_>>();
        let field = |name: &str| {
            fields
                .iter()
                .position(|field| field == name)
                .and_then(|index| parts.get(index).copied())
        };
        let start = parse_ass_timestamp(
            field("start").ok_or_else(|| MediaError::invalid_input("ASS cue has no start time"))?,
        )?;
        let end = parse_ass_timestamp(
            field("end").ok_or_else(|| MediaError::invalid_input("ASS cue has no end time"))?,
        )?;
        validate_range(start, end)?;
        let payload = field("text").unwrap_or_default();
        if payload.trim().is_empty() {
            continue;
        }
        let settings = fields
            .iter()
            .zip(parts.iter())
            .filter(|(name, _)| {
                name.as_str() != "start" && name.as_str() != "end" && name.as_str() != "text"
            })
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join(",");
        cues.push(SubtitleCue {
            id: None,
            start,
            end,
            text: normalize_ass_text(payload).into(),
            raw: line.to_owned().into(),
            settings: (!settings.is_empty()).then(|| settings.into()),
            format: SubtitleFormat::Ass,
        });
    }
    require_cues(cues, SubtitleFormat::Ass)
}

fn parse_timing_line(
    line: &str,
    webvtt: bool,
) -> MediaResult<(Duration, Duration, Option<String>)> {
    let (start, remainder) = line
        .split_once("-->")
        .ok_or_else(|| MediaError::invalid_input("subtitle cue has no timing separator"))?;
    let mut end_and_settings = remainder.split_whitespace();
    let end = end_and_settings
        .next()
        .ok_or_else(|| MediaError::invalid_input("subtitle cue has no end time"))?;
    let start = parse_clock_timestamp(start.trim(), webvtt)?;
    let end = parse_clock_timestamp(end.trim(), webvtt)?;
    validate_range(start, end)?;
    let settings = end_and_settings.collect::<Vec<_>>().join(" ");
    Ok((start, end, (!settings.is_empty()).then_some(settings)))
}

fn parse_clock_timestamp(value: &str, webvtt: bool) -> MediaResult<Duration> {
    let value = if webvtt {
        value.replace(',', ".")
    } else {
        value.replace(',', ".")
    };
    let parts = value.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return Err(MediaError::invalid_input(format!(
            "invalid subtitle timestamp {value:?}"
        )));
    }
    let (hours, minutes, seconds) = if parts.len() == 3 {
        (
            parse_u64(parts[0], value.as_str())?,
            parse_u64(parts[1], value.as_str())?,
            parts[2],
        )
    } else {
        (0, parse_u64(parts[0], value.as_str())?, parts[1])
    };
    duration_from_clock(hours, minutes, seconds, value.as_str())
}

fn parse_ass_timestamp(value: &str) -> MediaResult<Duration> {
    let parts = value.trim().split(':').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(MediaError::invalid_input(format!(
            "invalid ASS timestamp {value:?}"
        )));
    }
    duration_from_clock(
        parse_u64(parts[0], value)?,
        parse_u64(parts[1], value)?,
        parts[2],
        value,
    )
}

fn duration_from_clock(
    hours: u64,
    minutes: u64,
    seconds: &str,
    original: &str,
) -> MediaResult<Duration> {
    let seconds = seconds.parse::<f64>().map_err(|_| {
        MediaError::invalid_input(format!("invalid subtitle timestamp {original:?}"))
    })?;
    if minutes >= 60 || !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
        return Err(MediaError::invalid_input(format!(
            "invalid subtitle timestamp {original:?}"
        )));
    }
    let whole_seconds = hours
        .checked_mul(3600)
        .and_then(|value| value.checked_add(minutes * 60))
        .ok_or_else(|| MediaError::invalid_input("subtitle timestamp is too large"))?;
    Duration::from_secs(whole_seconds)
        .checked_add(Duration::from_secs_f64(seconds))
        .ok_or_else(|| MediaError::invalid_input("subtitle timestamp is too large"))
}

fn parse_u64(value: &str, original: &str) -> MediaResult<u64> {
    value
        .parse::<u64>()
        .map_err(|_| MediaError::invalid_input(format!("invalid subtitle timestamp {original:?}")))
}

fn validate_range(start: Duration, end: Duration) -> MediaResult<()> {
    if end < start {
        Err(MediaError::invalid_input(
            "subtitle cue ends before it starts",
        ))
    } else {
        Ok(())
    }
}

fn require_cues(cues: Vec<SubtitleCue>, format: SubtitleFormat) -> MediaResult<Vec<SubtitleCue>> {
    if cues.is_empty() {
        Err(MediaError::invalid_input(format!(
            "no {format:?} subtitle cues were found"
        )))
    } else {
        Ok(cues)
    }
}

fn normalize_newlines(source: &str) -> String {
    source.replace("\r\n", "\n").replace('\r', "\n")
}

fn split_blocks(source: &str) -> impl Iterator<Item = &str> {
    source
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
}

/// Removes markup tags and decodes basic entities in an embedded subtitle cue.
pub fn normalize_subtitle_text(source: &str) -> String {
    normalize_markup_text(source)
}

fn normalize_markup_text(source: &str) -> String {
    let mut text = String::with_capacity(source.len());
    let mut in_tag = false;
    for character in source.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    decode_basic_entities(&text)
}

fn normalize_ass_text(source: &str) -> String {
    let mut text = String::with_capacity(source.len());
    let mut override_depth = 0_u32;
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '{' => override_depth = override_depth.saturating_add(1),
            '}' if override_depth > 0 => override_depth -= 1,
            '\\' if override_depth == 0 => match characters.peek().copied() {
                Some('N' | 'n') => {
                    characters.next();
                    text.push('\n');
                }
                Some('h') => {
                    characters.next();
                    text.push(' ');
                }
                _ => text.push(character),
            },
            _ if override_depth == 0 => text.push(character),
            _ => {}
        }
    }
    text
}

fn decode_basic_entities(source: &str) -> String {
    source
        .replace("&nbsp;", "\u{00a0}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn strip_ascii_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .then(|| value[prefix.len()..].trim_start())
}

fn default_ass_fields() -> Vec<String> {
    [
        "layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SubtitleFormat, detect_subtitle_format, parse_subtitles};

    #[test]
    fn parses_subrip_and_normalizes_markup() {
        let parsed = parse_subtitles(
            "1\r\n00:00:01,250 --> 00:00:03,500\r\n<i>Hello</i> &amp; goodbye\r\n",
            None,
        )
        .unwrap();

        assert_eq!(parsed.format, SubtitleFormat::SubRip);
        assert_eq!(parsed.cues.len(), 1);
        assert_eq!(parsed.cues[0].start, Duration::from_millis(1250));
        assert_eq!(parsed.cues[0].end, Duration::from_millis(3500));
        assert_eq!(parsed.cues[0].text.as_ref(), "Hello & goodbye");
        assert!(parsed.cues[0].raw.contains("00:00:01,250"));
    }

    #[test]
    fn parses_webvtt_settings_and_identifier() {
        let parsed = parse_subtitles(
            "WEBVTT\n\nintro\n00:01.000 --> 00:03.000 line:90% align:center\n<v Alice>Hello</v>\n",
            None,
        )
        .unwrap();

        assert_eq!(parsed.format, SubtitleFormat::WebVtt);
        assert_eq!(parsed.cues[0].text.as_ref(), "Hello");
        assert_eq!(
            parsed.cues[0].settings.as_deref(),
            Some("line:90% align:center")
        );
    }

    #[test]
    fn parses_ass_dialogue_and_preserves_fields() {
        let parsed = parse_subtitles(
            "[Script Info]\nTitle: Example\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:02.00,0:00:04.50,Default,Alice,0,0,0,,{\\i1}Hello\\Nworld\n",
            None,
        )
        .unwrap();

        assert_eq!(parsed.format, SubtitleFormat::Ass);
        assert_eq!(parsed.cues[0].start, Duration::from_secs(2));
        assert_eq!(parsed.cues[0].end, Duration::from_millis(4500));
        assert_eq!(parsed.cues[0].text.as_ref(), "Hello\nworld");
        assert!(
            parsed.cues[0]
                .settings
                .as_deref()
                .is_some_and(|settings| settings.contains("style=Default"))
        );
    }

    #[test]
    fn detects_supported_formats() {
        assert_eq!(
            detect_subtitle_format("WEBVTT\n\n00:00.000 --> 00:01.000\ntext"),
            Some(SubtitleFormat::WebVtt)
        );
        assert_eq!(
            detect_subtitle_format("1\n00:00:00,000 --> 00:00:01,000\ntext"),
            Some(SubtitleFormat::SubRip)
        );
        assert_eq!(
            detect_subtitle_format(
                "[Events]\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,text"
            ),
            Some(SubtitleFormat::Ass)
        );
    }

    #[test]
    fn rejects_reversed_ranges() {
        let error = parse_subtitles(
            "1\n00:00:02,000 --> 00:00:01,000\ntext",
            Some(SubtitleFormat::SubRip),
        )
        .unwrap_err();

        assert!(error.to_string().contains("ends before"));
    }
}
