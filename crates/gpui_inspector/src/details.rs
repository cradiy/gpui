use gpui::{
    Background, Bounds, Div, Edges, InspectorElement, Length, Pixels, Size, div, prelude::*, px,
    rgb,
};

use crate::{definite, edges, property};

pub(super) struct Property {
    label: &'static str,
    value: String,
    swatch: Option<Background>,
}

fn row(label: &'static str, value: impl Into<String>) -> Property {
    Property {
        label,
        value: value.into(),
        swatch: None,
    }
}

fn color_hex(value: gpui::Hsla) -> String {
    let value = gpui::Rgba::from(value);
    let [r, g, b, a] = [value.r, value.g, value.b, value.a]
        .map(|channel| (channel.clamp(0., 1.) * 255.).round() as u8);
    format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}

fn color(label: &'static str, value: gpui::Hsla) -> Property {
    Property {
        label,
        value: color_hex(value),
        swatch: Some(value.into()),
    }
}

pub(super) struct Section {
    title: &'static str,
    rows: Vec<Property>,
}

pub(super) fn content_size(element: &InspectorElement) -> Size<Pixels> {
    let b = &element.box_model;
    gpui::size(
        (element.bounds.size.width
            - b.border.left
            - b.border.right
            - b.padding.left
            - b.padding.right
            - b.scrollbar.width)
            .max(px(0.)),
        (element.bounds.size.height
            - b.border.top
            - b.border.bottom
            - b.padding.top
            - b.padding.bottom
            - b.scrollbar.height)
            .max(px(0.)),
    )
}

fn length(value: Length) -> String {
    match value {
        Length::Auto => "auto".into(),
        Length::Definite(value) => definite(value),
    }
}

fn size(value: Size<Pixels>) -> String {
    format!(
        "{:.1} × {:.1} px",
        f32::from(value.width),
        f32::from(value.height)
    )
}

fn bounds(value: Bounds<Pixels>) -> String {
    format!(
        "{:.1}, {:.1} · {}",
        f32::from(value.origin.x),
        f32::from(value.origin.y),
        size(value.size)
    )
}

fn configured_size(value: Size<Length>) -> String {
    format!("{} × {}", length(value.width), length(value.height))
}

fn optional<T: std::fmt::Debug>(value: Option<T>) -> String {
    value
        .map(|value| format!("{value:?}"))
        .unwrap_or_else(|| "default".into())
}

pub(super) fn sections(
    element: &InspectorElement,
    parent: Option<&InspectorElement>,
    tab: usize,
) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut push = |title, rows| sections.push(Section { title, rows });
    match tab {
        0 => {
            let mut rows = vec![
                row("Border box", size(element.bounds.size)),
                row("Content box", size(content_size(element))),
                row(
                    "Window x / y",
                    format!(
                        "{:.1}, {:.1}",
                        f32::from(element.bounds.origin.x),
                        f32::from(element.bounds.origin.y)
                    ),
                ),
            ];
            if let Some(parent) = parent {
                let delta = element.bounds.origin - parent.bounds.origin;
                rows.push(row(
                    "Parent x / y",
                    format!("{:.1}, {:.1}", f32::from(delta.x), f32::from(delta.y)),
                ));
            }
            push("MEASURED", rows);
            if let Some(style) = &element.style {
                push(
                    "CONSTRAINTS",
                    vec![
                        row("Width / height", configured_size(style.size)),
                        row("Min size", configured_size(style.min_size)),
                        row("Max size", configured_size(style.max_size)),
                        row("Aspect ratio", optional(style.aspect_ratio)),
                        row("Display", format!("{:?}", style.display)),
                        row("Position", format!("{:?}", style.position)),
                        row("Inset T R B L", edges(style.inset, length)),
                    ],
                );
                push(
                    "LAYOUT",
                    vec![
                        row("Direction", format!("{:?}", style.flex_direction)),
                        row("Wrap", format!("{:?}", style.flex_wrap)),
                        row("Justify", optional(style.justify_content)),
                        row("Align items", optional(style.align_items)),
                        row("Align self", optional(style.align_self)),
                        row("Align content", optional(style.align_content)),
                        row(
                            "Grow / shrink",
                            format!("{} / {}", style.flex_grow, style.flex_shrink),
                        ),
                        row("Flex basis", length(style.flex_basis)),
                        row(
                            "Column / row gap",
                            format!(
                                "{} / {}",
                                definite(style.gap.width),
                                definite(style.gap.height)
                            ),
                        ),
                    ],
                );
                if style.grid_cols.is_some()
                    || style.grid_rows.is_some()
                    || style.grid_location.is_some()
                {
                    push(
                        "GRID",
                        vec![
                            row("Columns", format!("{:?}", style.grid_cols)),
                            row("Rows", format!("{:?}", style.grid_rows)),
                            row("Placement", format!("{:?}", style.grid_location)),
                        ],
                    );
                }
                push(
                    "SPACING · CONFIGURED",
                    vec![
                        row("Margin T R B L", edges(style.margin, length)),
                        row("Padding T R B L", edges(style.padding, definite)),
                    ],
                );
            }
            let visible = element.bounds.intersect(&element.content_mask.bounds);
            let mut rows = vec![
                row("Visible rect", bounds(visible)),
                row("Ancestor clip", bounds(element.content_mask.bounds)),
                row("Reserved scrollbar", size(element.box_model.scrollbar)),
            ];
            if let Some(style) = &element.style {
                rows.push(row(
                    "Overflow x / y",
                    format!("{:?} / {:?}", style.overflow.x, style.overflow.y),
                ));
            }
            push("CLIPPING", rows);
        }
        1 => {
            if let Some(style) = &element.style {
                let background = style.background.as_ref().and_then(|fill| fill.color());
                let mut rows = vec![
                    Property {
                        label: "Background",
                        value: background
                            .map(|bg| {
                                bg.as_solid()
                                    .map(color_hex)
                                    .unwrap_or_else(|| "Gradient / pattern".into())
                            })
                            .unwrap_or_else(|| "None".into()),
                        swatch: background,
                    },
                    color("Text color", element.text_style.color),
                    row(
                        "Local opacity",
                        format!("{:.0}%", style.opacity.unwrap_or(1.) * 100.),
                    ),
                    row("Visibility", format!("{:?}", style.visibility)),
                ];
                rows.push(row(
                    "Text color source",
                    if style.text.color.is_some() {
                        "Element"
                    } else {
                        "Inherited"
                    },
                ));
                push("COLORS", rows);
                let mut rows = vec![
                    row(
                        "Width T R B L",
                        edges(element.box_model.border, |value| {
                            format!("{:.1}px", f32::from(value))
                        }),
                    ),
                    row("Style", format!("{:?}", style.border_style)),
                ];
                if let Some(value) = style.border_color {
                    rows.push(color("Color", value));
                }
                for (label, value) in [
                    ("Top color", style.border_top_color),
                    ("Right color", style.border_right_color),
                    ("Bottom color", style.border_bottom_color),
                    ("Left color", style.border_left_color),
                ] {
                    if let Some(value) = value {
                        rows.push(color(label, value));
                    }
                }
                let radii = style.corner_radii.to_pixels(element.rem_size);
                rows.push(row(
                    "Radius TL TR BR BL",
                    format!(
                        "{:.1} · {:.1} · {:.1} · {:.1} px",
                        f32::from(radii.top_left),
                        f32::from(radii.top_right),
                        f32::from(radii.bottom_right),
                        f32::from(radii.bottom_left)
                    ),
                ));
                if style.border_gradient.is_some() {
                    rows.push(row(
                        "Border gradient",
                        format!("{:?}", style.border_gradient),
                    ));
                }
                push("BORDER", rows);
                let mut rows = vec![
                    row(
                        "Backdrop blur",
                        style
                            .backdrop_blur
                            .map(|value| format!("{:.1}px", f32::from(value)))
                            .unwrap_or_else(|| "None".into()),
                    ),
                    row("Shadows", style.box_shadow.len().to_string()),
                ];
                for shadow in &style.box_shadow {
                    rows.push(row("Shadow", format!("{shadow:?}")));
                }
                push("EFFECTS", rows);
            } else {
                push(
                    "STYLE",
                    vec![row(
                        "Availability",
                        "No Interactivity style on this element",
                    )],
                );
            }
        }
        2 => {
            let style = &element.text_style;
            let font_size = style.font_size.to_pixels(element.rem_size);
            push(
                "RESOLVED BASE TEXT STYLE",
                vec![
                    row("Font family", style.font_family.to_string()),
                    row(
                        "Font size",
                        format!("{:.1}px ({})", f32::from(font_size), style.font_size),
                    ),
                    row(
                        "Line height",
                        format!(
                            "{:.1}px",
                            f32::from(
                                style
                                    .line_height
                                    .to_pixels(font_size.into(), element.rem_size)
                            )
                        ),
                    ),
                    row(
                        "Weight / style",
                        format!("{} / {:?}", style.font_weight.0, style.font_style),
                    ),
                    color("Color", style.color),
                    row("Align", format!("{:?}", style.text_align)),
                    row("Whitespace", format!("{:?}", style.white_space)),
                    row("Overflow", format!("{:?}", style.text_overflow)),
                    row("Line clamp", optional(style.line_clamp)),
                    row("Rem size", format!("{:.1}px", f32::from(element.rem_size))),
                    row("Fallbacks", format!("{:?}", style.font_fallbacks)),
                ],
            );
            if element.text.is_empty() {
                push(
                    "DIRECT TEXT",
                    vec![row(
                        "Content",
                        "No direct text. Select a child element to inspect its content.",
                    )],
                );
            }
            for text in &element.text {
                push(
                    "DIRECT TEXT",
                    vec![
                        row(
                            "Preview",
                            format!(
                                "{}{}",
                                text.preview,
                                if text.preview_shortened { "…" } else { "" }
                            ),
                        ),
                        row("Bounds", bounds(text.bounds)),
                        row(
                            "Line height",
                            format!("{:.1}px", f32::from(text.line_height)),
                        ),
                        row("Base font", text.base_style.font_family.to_string()),
                        color("Base color", text.base_style.color),
                    ],
                );
            }
            push(
                "TEXT SCOPE",
                vec![
                    row(
                        "Styled spans",
                        "Per-span font and color overrides are not listed.",
                    ),
                    row(
                        "Font fallback",
                        "Family and fallbacks describe the requested font stack, not the resolved face of each glyph.",
                    ),
                ],
            );
        }
        _ => {
            let source = element.id.path.source_location;
            push(
                "ELEMENT",
                vec![
                    row("Rust type", element.type_name),
                    row("Global ID", element.id.path.global_id.to_string()),
                    row("Instance", element.id.instance_id.to_string()),
                    row(
                        "Source",
                        format!("{}:{}:{}", source.file(), source.line(), source.column()),
                    ),
                ],
            );
        }
    }
    sections
}

pub(super) fn render(sections: Vec<Section>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_4()
        .children(sections.into_iter().map(|section| {
            div()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x94a3b8))
                        .mb_2()
                        .child(section.title),
                )
                .children(section.rows.into_iter().map(|row| {
                    let mut element = property(row.label, row.value);
                    if let Some(swatch) = row.swatch {
                        element = element.child(
                            div()
                                .size(px(16.))
                                .flex_shrink_0()
                                .rounded_sm()
                                .border_1()
                                .border_color(rgb(0x64748b))
                                .bg(swatch),
                        );
                    }
                    element
                }))
        }))
}

pub(super) fn report(element: &InspectorElement, parent: Option<&InspectorElement>) -> String {
    let mut result = String::from("GPUI Inspector\n");
    for tab in 0..4 {
        for section in sections(element, parent, tab) {
            result.push_str(&format!("\n{}\n", section.title));
            for row in section.rows {
                result.push_str(&format!("{}: {}\n", row.label, row.value));
            }
        }
    }
    for (name, values) in [
        ("Resolved margin", element.box_model.margin),
        ("Resolved border", element.box_model.border),
        ("Resolved padding", element.box_model.padding),
    ] {
        result.push_str(&format!(
            "{name} T R B L: {}\n",
            edges(values, |value| format!("{:.1}px", f32::from(value)))
        ));
    }
    result
}

fn layer(label: &'static str, values: Edges<Pixels>, color: u32, inner: Div) -> Div {
    let value = |value: Pixels| format!("{:.1}", f32::from(value));
    div()
        .border_1()
        .border_color(rgb(color))
        .bg(gpui::rgba((color << 8) | 0x20))
        .px_2()
        .py_1()
        .child(
            div()
                .flex()
                .justify_between()
                .text_xs()
                .child(label)
                .child(value(values.top)),
        )
        .child(
            div()
                .flex()
                .gap_2()
                .items_center()
                .child(div().text_xs().child(value(values.left)))
                .child(inner.flex_1().min_w_0())
                .child(div().text_xs().child(value(values.right))),
        )
        .child(div().text_xs().text_center().child(value(values.bottom)))
}

pub(super) fn box_model(element: &InspectorElement) -> Div {
    let model = &element.box_model;
    let content = div()
        .py_2()
        .text_center()
        .bg(rgb(0x25466a))
        .child(size(content_size(element)));
    div()
        .mb_4()
        .child(layer(
            "margin",
            model.margin,
            0xa67a4a,
            layer(
                "border",
                model.border,
                0xaaa16a,
                layer("padding", model.padding, 0x5b9367, content),
            ),
        ))
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(rgb(0x94a3b8))
                .child("Resolved layout pixels · center is the content box"),
        )
}
