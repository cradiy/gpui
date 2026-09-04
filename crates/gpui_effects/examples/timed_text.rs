use std::time::Duration;

use gpui::{
    Animation, AnimationExt, App, Background, Bounds, Context, FontWeight, Render, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, rgba, size,
};
use gpui_effects::{
    FrostedGlass, FrostedGlassAppearance, TimedText, TimedTextRevealWave, TimedTextUnit,
};
use gpui_platform::application;

type Piece = (&'static str, u64, u64, usize);

const CHINESE_LINE: &str = "长音符短长旋律快长回声啪";
const ENGLISH_LINE: &str = "sustain  short  melody  tap  long echo  pop";
const MIXED_LINE: &str = "长音符short长旋律tap long echo快";
const LOOP_SECONDS: f32 = 6.0;

const CHINESE_PIECES: &[Piece] = &[
    ("长音符", 0, 1500, 0),
    ("短", 1500, 1850, 1),
    ("长旋律", 1850, 3450, 2),
    ("快", 3450, 3750, 3),
    ("长回声", 3750, 5750, 4),
    ("啪", 5750, 6000, 5),
];

const ENGLISH_PIECES: &[Piece] = &[
    ("sustain", 0, 1500, 0),
    ("short", 1500, 1850, 1),
    ("melody", 1850, 3450, 2),
    ("tap", 3450, 3750, 3),
    ("long", 3750, 4550, 4),
    ("echo", 4550, 5750, 5),
    ("pop", 5750, 6000, 6),
];

const MIXED_PIECES: &[Piece] = &[
    ("长音符", 0, 1500, 0),
    ("short", 1500, 1850, 1),
    ("长旋律", 1850, 3450, 2),
    ("tap", 3450, 3750, 3),
    ("long", 3750, 4550, 4),
    ("echo", 4550, 5750, 5),
    ("快", 5750, 6000, 6),
];

fn timings(line: &'static str, pieces: &'static [Piece]) -> Vec<TimedTextUnit> {
    // Units can be graphemes, words, or arbitrary UTF-8 ranges. Characters in
    // the same group reveal independently but lift as one phrase.
    let mut search_from = 0;
    pieces
        .iter()
        .copied()
        .map(|(piece, start, end, group)| {
            let relative = line[search_from..]
                .find(piece)
                .expect("timed fragment must occur in order");
            let range_start = search_from + relative;
            let range_end = range_start + piece.len();
            search_from = range_end;
            TimedTextUnit::new(
                range_start..range_end,
                Duration::from_millis(start),
                Duration::from_millis(end),
            )
            .group(group)
        })
        .collect()
}

fn timed_row(
    label: &'static str,
    animation_id: &'static str,
    line: &'static str,
    pieces: &'static [Piece],
    fill: Background,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .w(px(54.))
                .text_xs()
                .text_color(rgba(0xe7edf870))
                .child(label),
        )
        .child(
            TimedText::new(line, timings(line, pieces))
                .active_fill(fill)
                .inactive_opacity(0.25)
                .reveal_wave(TimedTextRevealWave {
                    width: px(12.),
                    leading_opacity: 0.18,
                    softness: px(7.),
                })
                .progressive_lift(px(3.))
                .text_color(rgb(0xe7edf8))
                .text_size(px(28.))
                .font_weight(FontWeight::SEMIBOLD)
                .with_animation(
                    animation_id,
                    Animation::new(Duration::from_secs_f32(LOOP_SECONDS)).repeat(),
                    |line, phase| line.position(Duration::from_secs_f32(phase * LOOP_SECONDS)),
                ),
        )
}

struct TimedTextExample;

impl Render for TimedTextExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let fill: Background = rgb(0x67c7e9).into();
        let mut glass = FrostedGlassAppearance::dark();
        glass.tint = rgba(0x11182a80).into();

        div()
            .size_full()
            .text_color(rgb(0xe7edf8))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x090d18))
            .child(
                FrostedGlass::with_appearance(glass)
                    .opacity(0.76)
                    .w(px(900.))
                    .rounded(px(28.))
                    .px(px(42.))
                    .py(px(48.))
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .text_sm()
                            .child(
                                div()
                                    .text_color(rgb(0x8be9fd))
                                    .child("PROGRESSIVE WORD LIFT"),
                            )
                            .child(
                                div()
                                    .text_color(rgba(0xe7edf880))
                                    .child("RISES DURING PLAYBACK · HOLDS AT WORD END"),
                            )
                            .child(
                                div()
                                    .text_color(rgba(0xe7edf860))
                                    .child("SEGMENTATION  ·  EXPLICIT UTF-8 RANGES"),
                            ),
                    )
                    .child(timed_row(
                        "中文",
                        "timed-text-chinese",
                        CHINESE_LINE,
                        CHINESE_PIECES,
                        fill,
                    ))
                    .child(timed_row(
                        "EN",
                        "timed-text-english",
                        ENGLISH_LINE,
                        ENGLISH_PIECES,
                        fill,
                    ))
                    .child(timed_row(
                        "混合",
                        "timed-text-mixed",
                        MIXED_LINE,
                        MIXED_PIECES,
                        fill,
                    )),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(620.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TimedTextExample),
        )
        .expect("failed to open timed text example");
    });
}
