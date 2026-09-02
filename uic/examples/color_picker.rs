use anyhow::{Context as _, bail};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, Entity, Hsla, IntoElement, Render, Rgba, Role,
    SharedString, Subscription, Window, WindowBounds, WindowOptions, checkerboard, div, hsla,
    prelude::*, px, rgb, rgba, size,
};
use uic::components::{
    color_picker::{
        AlphaSlider, ColorPicker, ColorPickerAppearance, ColorPickerEvent, ColorPickerState,
        ColorPickerTrigger, ColorPickerTriggerSize, Hsva,
    },
    input::{Input, InputAppearance, InputEvent, TextInput},
    popover::{Popover, PopoverPlacement, PopoverState},
};

// Everything except the SV + Hue `ColorPicker` is composed at the consumer layer in this example.
const MATERIAL_COLORS: [u32; 48] = [
    0xf44336, 0xe91e63, 0x9c27b0, 0x673ab7, 0x3f51b5, 0x2196f3, 0x03a9f4, 0x00bcd4, 0x009688,
    0x4caf50, 0x8bc34a, 0xcddc39, 0xffeb3b, 0xffc107, 0xff9800, 0xff5722, 0xc2185b, 0x7b1fa2,
    0x512da8, 0x303f9f, 0x1976d2, 0x0288d1, 0x0097a7, 0x00796b, 0x388e3c, 0x689f38, 0xafb42b,
    0xfbc02d, 0xffa000, 0xf57c00, 0xe64a19, 0xd32f2f, 0x880e4f, 0x4a148c, 0x311b92, 0x1a237e,
    0x0d47a1, 0x01579b, 0x006064, 0x004d40, 0x1b5e20, 0x33691e, 0x827717, 0xf57f17, 0xff6f00,
    0xe65100, 0xbf360c, 0xb71c1c,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorFormat {
    Hex,
    Rgb,
    Hsv,
    Hsl,
}

impl ColorFormat {
    const ALL: [Self; 4] = [Self::Hex, Self::Rgb, Self::Hsv, Self::Hsl];

    fn label(self) -> &'static str {
        match self {
            Self::Hex => "Hex",
            Self::Rgb => "RGB",
            Self::Hsv => "HSV",
            Self::Hsl => "HSL",
        }
    }
}

struct ColorPickerExample {
    picker: Entity<ColorPickerState>,
    popup_picker: Entity<ColorPickerState>,
    popover: Entity<PopoverState>,
    hex_input: Entity<TextInput>,
    rgb_input: Entity<TextInput>,
    hsv_input: Entity<TextInput>,
    hsl_input: Entity<TextInput>,
    invalid_format: Option<ColorFormat>,
    pending_input_changes: Vec<(ColorFormat, SharedString)>,
    _subscriptions: Vec<Subscription>,
}

impl ColorPickerExample {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = rgba(0x20222eff);
        let hsva = Hsva::from(initial);
        let picker = cx.new(|cx| ColorPickerState::new(initial, cx));
        let hex_input = cx.new(|cx| TextInput::new(cx).initial_value(format_hex(initial)));
        let rgb_input = cx.new(|cx| TextInput::new(cx).initial_value(format_rgb(initial)));
        let hsv_input = cx.new(|cx| TextInput::new(cx).initial_value(format_hsv(hsva)));
        let hsl_input = cx.new(|cx| TextInput::new(cx).initial_value(format_hsl(hsva)));

        let picker_subscription = cx.subscribe(&picker, |this, _, event: &ColorPickerEvent, cx| {
            if matches!(event, ColorPickerEvent::Commit(_)) {
                this.sync_inputs(None, cx);
                cx.notify();
            }
        });
        let hex_subscription = cx.subscribe(&hex_input, |this, _, event: &InputEvent, cx| {
            this.handle_input(ColorFormat::Hex, event, cx);
        });
        let rgb_subscription = cx.subscribe(&rgb_input, |this, _, event: &InputEvent, cx| {
            this.handle_input(ColorFormat::Rgb, event, cx);
        });
        let hsv_subscription = cx.subscribe(&hsv_input, |this, _, event: &InputEvent, cx| {
            this.handle_input(ColorFormat::Hsv, event, cx);
        });
        let hsl_subscription = cx.subscribe(&hsl_input, |this, _, event: &InputEvent, cx| {
            this.handle_input(ColorFormat::Hsl, event, cx);
        });
        Self {
            picker,
            popup_picker: cx.new(|cx| ColorPickerState::new(initial, cx)),
            popover: cx.new(|cx| PopoverState::new(window, cx)),
            hex_input,
            rgb_input,
            hsv_input,
            hsl_input,
            invalid_format: None,
            pending_input_changes: Vec::new(),
            _subscriptions: vec![
                picker_subscription,
                hex_subscription,
                rgb_subscription,
                hsv_subscription,
                hsl_subscription,
            ],
        }
    }

    fn input(&self, format: ColorFormat) -> &Entity<TextInput> {
        match format {
            ColorFormat::Hex => &self.hex_input,
            ColorFormat::Rgb => &self.rgb_input,
            ColorFormat::Hsv => &self.hsv_input,
            ColorFormat::Hsl => &self.hsl_input,
        }
    }

    fn handle_input(&mut self, format: ColorFormat, event: &InputEvent, cx: &mut Context<Self>) {
        if let InputEvent::Change(value) = event
            && let Some(index) =
                self.pending_input_changes
                    .iter()
                    .position(|(pending_format, pending_value)| {
                        *pending_format == format && pending_value == value
                    })
        {
            self.pending_input_changes.remove(index);
            return;
        }

        match event {
            InputEvent::Change(value) => {
                if let Ok(value) = self.parse(format, value, cx) {
                    self.set_value(value, Some(format), cx);
                } else if self.invalid_format == Some(format) {
                    self.invalid_format = None;
                    cx.notify();
                }
            }
            InputEvent::Submit(value) => match self.parse(format, value, cx) {
                Ok(value) => self.set_value(value, None, cx),
                Err(_) => {
                    self.invalid_format = Some(format);
                    cx.notify();
                }
            },
        }
    }

    fn parse(
        &self,
        format: ColorFormat,
        value: &str,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Rgba> {
        let alpha = self.picker.read(cx).value().a;
        match format {
            ColorFormat::Hex => parse_hex(value, alpha),
            ColorFormat::Rgb => parse_rgb(value, alpha),
            ColorFormat::Hsv => parse_hsv(value, alpha).map(Hsva::to_rgba),
            ColorFormat::Hsl => parse_hsl(value, alpha),
        }
    }

    fn set_value(&mut self, value: Rgba, excluded: Option<ColorFormat>, cx: &mut Context<Self>) {
        self.picker
            .update(cx, |picker, cx| picker.set_value(value, cx));
        self.invalid_format = None;
        self.sync_inputs(excluded, cx);
        cx.notify();
    }

    fn set_palette_color(&mut self, mut value: Rgba, cx: &mut Context<Self>) {
        value.a = self.picker.read(cx).value().a;
        self.set_value(value, None, cx);
    }

    fn sync_inputs(&mut self, excluded: Option<ColorFormat>, cx: &mut Context<Self>) {
        let hsva = self.picker.read(cx).hsva();
        let value = hsva.to_rgba();
        let fields = [
            (ColorFormat::Hex, self.hex_input.clone(), format_hex(value)),
            (ColorFormat::Rgb, self.rgb_input.clone(), format_rgb(value)),
            (ColorFormat::Hsv, self.hsv_input.clone(), format_hsv(hsva)),
            (ColorFormat::Hsl, self.hsl_input.clone(), format_hsl(hsva)),
        ];
        for (format, input, value) in fields {
            if excluded == Some(format) || input.read(cx).value().as_ref() == value {
                continue;
            }
            let value = SharedString::from(value);
            self.pending_input_changes.push((format, value.clone()));
            input.update(cx, |input, cx| input.set_value(value, cx));
        }
    }

    fn palette(
        &self,
        label: &'static str,
        colors: impl IntoIterator<Item = Rgba>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.picker.read(cx).value();
        let mut swatches = div().flex().flex_wrap().gap_1();
        for (index, color) in colors.into_iter().enumerate() {
            let is_selected = rgb_key(color) == rgb_key(selected);
            swatches = swatches.child(
                div()
                    .id(SharedString::from(format!("{label}-{index}")))
                    .focusable()
                    .tab_stop(true)
                    .role(Role::Button)
                    .aria_label(format!("Choose {}", format_hex(color)))
                    .size(px(38.0))
                    .rounded_md()
                    .border(if is_selected { px(2.0) } else { px(1.0) })
                    .border_color(if is_selected {
                        rgb(0xffffff)
                    } else {
                        rgb(0x3f4648)
                    })
                    .bg(color)
                    .cursor_pointer()
                    .focus_visible(|style| style.border_2().border_color(rgb(0x06b6d4)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_palette_color(color, cx);
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().text_color(rgb(0xe5e7eb)).child(label))
            .child(swatches)
    }

    fn alpha_slider(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        AlphaSlider::new(&self.picker)
    }

    fn fields(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hsva = self.picker.read(cx).hsva();
        let value = hsva.to_rgba();
        let mut fields = div().flex().flex_wrap().gap_3();
        for format in ColorFormat::ALL {
            let input = self.input(format).clone();
            let copy_text = match format {
                ColorFormat::Hex => format_hex(value),
                ColorFormat::Rgb => format_rgb(value),
                ColorFormat::Hsv => format_hsv(hsva),
                ColorFormat::Hsl => format_hsl(hsva),
            };
            let appearance = example_input_appearance();
            let border = if self.invalid_format == Some(format) {
                hsla(0.0, 0.75, 0.55, 1.0)
            } else {
                hsla(0.52, 0.08, 0.26, 1.0)
            };
            fields = fields.child(
                div()
                    .min_w(px(205.0))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x9ca3af))
                            .child(format.label()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Input::new(&input)
                                    .appearance(appearance)
                                    .h(px(40.0))
                                    .rounded(px(9.0))
                                    .border_color(border)
                                    .bg(hsla(0.52, 0.12, 0.09, 1.0))
                                    .text_color(hsla(0.0, 0.0, 0.9, 1.0))
                                    .text_size(px(15.0)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "example-copy-{}",
                                        format.label()
                                    )))
                                    .focusable()
                                    .tab_stop(true)
                                    .role(Role::Button)
                                    .aria_label(format!("Copy {} color", format.label()))
                                    .px_2()
                                    .h(px(40.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .text_xs()
                                    .text_color(rgb(0x9ca3af))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0x31383a).opacity(0.6)))
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_text.clone(),
                                        ));
                                    })
                                    .child("Copy"),
                            ),
                    ),
            );
        }
        fields
    }
}

impl Render for ColorPickerExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let value = self.picker.read(cx).value();
        let popup_value = self.popup_picker.read(cx).value();
        let popover_open = self.popover.read(cx).is_open();
        let popup_picker = self.popup_picker.clone();
        div()
            .id("color-picker-example-scroll")
            .size_full()
            .overflow_y_scroll()
            .p_8()
            .bg(rgb(0x090b0c))
            .child(
                div()
                    .w_full()
                    .max_w(px(960.0))
                    .mx_auto()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .text_2xl()
                            .text_color(rgb(0xe5e7eb))
                            .child("Color Picker Composition"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x9ca3af))
                            .child("Only the SV canvas and hue track come from ColorPicker."),
                    )
                    .child(
                        div()
                            .p_4()
                            .rounded_lg()
                            .border_1()
                            .border_color(rgb(0x303437))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x9ca3af))
                                    .child("Popover triggers"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        ColorPickerTrigger::new("small-color-trigger", popup_value)
                                            .control_size(ColorPickerTriggerSize::Small),
                                    )
                                    .child(
                                        Popover::new(&self.popover)
                                            .label("Choose color")
                                            .placement(PopoverPlacement::BottomStart)
                                            .trigger(
                                                ColorPickerTrigger::new(
                                                    "interactive-color-trigger",
                                                    popup_value,
                                                )
                                                .show_value(true)
                                                .active(popover_open),
                                            )
                                            .content(move |_, _| {
                                                div()
                                                    .w(px(390.))
                                                    .p_3()
                                                    .rounded_lg()
                                                    .bg(rgb(0x15191b))
                                                    .flex()
                                                    .flex_col()
                                                    .gap_3()
                                                    .child(
                                                        ColorPicker::new(&popup_picker)
                                                            .appearance(
                                                                ColorPickerAppearance::default()
                                                                    .area_height(px(220.))
                                                                    .hue_width(px(26.))
                                                                    .marker_size(px(16.)),
                                                            )
                                                            .p_0()
                                                            .border_0()
                                                            .bg(hsla(0.0, 0.0, 0.0, 0.0)),
                                                    )
                                                    .child(AlphaSlider::new(&popup_picker))
                                            })
                                            .p_0()
                                            .border_0()
                                            .bg(hsla(0.0, 0.0, 0.0, 0.0)),
                                    )
                                    .child(
                                        ColorPickerTrigger::new("large-color-trigger", popup_value)
                                            .control_size(ColorPickerTriggerSize::Large)
                                            .label(div().child("Custom color")),
                                    ),
                            ),
                    )
                    .child(
                        ColorPicker::new(&self.picker)
                            .sv_aria_label("Saturation and brightness")
                            .hue_aria_label("Hue"),
                    )
                    .child(self.palette("Material Colors", MATERIAL_COLORS.map(rgb), cx))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(rgb(0xe5e7eb))
                                            .child(format!("Opacity · {:.0}%", value.a * 100.0)),
                                    )
                                    .child(self.alpha_slider(cx)),
                            )
                            .child(
                                div()
                                    .w(px(76.0))
                                    .h(px(48.0))
                                    .rounded_lg()
                                    .border_1()
                                    .border_color(rgb(0x3f4648))
                                    .bg(checkerboard(hsla(0.0, 0.0, 0.45, 0.45), 6.0))
                                    .child(div().size_full().rounded_lg().bg(value)),
                            ),
                    )
                    .child(self.fields(cx)),
            )
    }
}

fn example_input_appearance() -> InputAppearance {
    InputAppearance::default()
        .placeholder(hsla(0.0, 0.0, 0.48, 1.0))
        .focus_border(hsla(0.52, 0.85, 0.48, 1.0))
        .caret(hsla(0.52, 0.85, 0.48, 1.0))
        .selection(hsla(0.52, 0.85, 0.48, 0.24))
}

fn format_hex(color: Rgba) -> String {
    let [r, g, b, _] = rgba_bytes(color);
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn format_rgb(color: Rgba) -> String {
    let [r, g, b, _] = rgba_bytes(color);
    format!("{r}, {g}, {b}")
}

fn format_hsv(color: Hsva) -> String {
    format!(
        "{:.0}°, {:.0}%, {:.0}%",
        color.h * 360.0,
        color.s * 100.0,
        color.v * 100.0
    )
}

fn format_hsl(color: Hsva) -> String {
    let mut hsl = Hsla::from(color.to_rgba());
    if hsl.s <= f32::EPSILON {
        hsl.h = color.h;
    }
    format!(
        "{:.0}°, {:.0}%, {:.0}%",
        hsl.h * 360.0,
        hsl.s * 100.0,
        hsl.l * 100.0
    )
}

fn parse_hex(value: &str, alpha: f32) -> anyhow::Result<Rgba> {
    let value = value.trim();
    let mut color = Rgba::try_from(value)?;
    if !matches!(value.len(), 5 | 9) {
        color.a = alpha;
    }
    Ok(color)
}

fn parse_rgb(value: &str, alpha: f32) -> anyhow::Result<Rgba> {
    let channels = split_channels(strip_function(value.trim(), "rgb"));
    if channels.len() != 3 {
        bail!("RGB requires exactly three channels");
    }
    let mut parsed = [0_u8; 3];
    for (target, channel) in parsed.iter_mut().zip(channels) {
        *target = channel
            .parse::<u8>()
            .with_context(|| format!("invalid RGB channel: {channel}"))?;
    }
    Ok(Rgba {
        r: f32::from(parsed[0]) / 255.0,
        g: f32::from(parsed[1]) / 255.0,
        b: f32::from(parsed[2]) / 255.0,
        a: alpha,
    })
}

fn parse_hsv(value: &str, alpha: f32) -> anyhow::Result<Hsva> {
    let channels = split_channels(strip_function(value.trim(), "hsv"));
    if channels.len() != 3 {
        bail!("HSV requires exactly three channels");
    }
    let hue = parse_number(channels[0], &["°", "deg"])?;
    let saturation = parse_number(channels[1], &["%"])?;
    let brightness = parse_number(channels[2], &["%"])?;
    if !(0.0..=360.0).contains(&hue)
        || !(0.0..=100.0).contains(&saturation)
        || !(0.0..=100.0).contains(&brightness)
    {
        bail!("HSV channels are out of range");
    }
    Ok(Hsva::new(
        hue / 360.0,
        saturation / 100.0,
        brightness / 100.0,
        alpha,
    ))
}

fn parse_hsl(value: &str, alpha: f32) -> anyhow::Result<Rgba> {
    let channels = split_channels(strip_function(value.trim(), "hsl"));
    if channels.len() != 3 {
        bail!("HSL requires exactly three channels");
    }
    let hue = parse_number(channels[0], &["°", "deg"])?;
    let saturation = parse_number(channels[1], &["%"])?;
    let lightness = parse_number(channels[2], &["%"])?;
    if !(0.0..=360.0).contains(&hue)
        || !(0.0..=100.0).contains(&saturation)
        || !(0.0..=100.0).contains(&lightness)
    {
        bail!("HSL channels are out of range");
    }
    Ok(Rgba::from(Hsla {
        h: hue / 360.0,
        s: saturation / 100.0,
        l: lightness / 100.0,
        a: alpha,
    }))
}

fn rgba_bytes(color: Rgba) -> [u8; 4] {
    [color.r, color.g, color.b, color.a]
        .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn rgb_key(color: Rgba) -> u32 {
    u32::from(color) >> 8
}

fn strip_function<'a>(value: &'a str, name: &str) -> &'a str {
    value
        .strip_prefix(name)
        .and_then(|value| value.strip_prefix('('))
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value)
}

fn split_channels(value: &str) -> Vec<&str> {
    value
        .split([',', ' '])
        .map(str::trim)
        .filter(|channel| !channel.is_empty())
        .collect()
}

fn parse_number(value: &str, suffixes: &[&str]) -> anyhow::Result<f32> {
    let mut value = value.trim();
    for suffix in suffixes {
        value = value.strip_suffix(suffix).unwrap_or(value).trim();
    }
    value
        .parse::<f32>()
        .with_context(|| format!("invalid numeric channel: {value}"))
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        uic::init(cx);
        let bounds = Bounds::centered(None, size(px(1040.0), px(960.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| ColorPickerExample::new(window, cx)),
        )
        .expect("failed to open color picker example window");
    });
}
