use gpui::{Context, Div, Inspector, InspectorHitbox, Pixels, Point, div, prelude::*, px, rgb};

use crate::theme;

fn status(hit: &InspectorHitbox) -> &'static str {
    if hit.captured {
        "Captured"
    } else if hit.mouse {
        "Mouse + scroll"
    } else if hit.scroll {
        "Scroll only"
    } else {
        "Occluded"
    }
}

pub(super) fn report(point: Point<Pixels>, hits: &[InspectorHitbox]) -> String {
    let mut result = format!(
        "POINTER PROBE\nWindow x / y: {:.1}, {:.1}\nFront to back:\n",
        f32::from(point.x),
        f32::from(point.y)
    );
    for hit in hits {
        result.push_str(&format!(
            "{}: {} · {:?} · {:?}\n",
            hit.element
                .as_ref()
                .map(|id| id.path.global_id.to_string())
                .unwrap_or_else(|| "Unattributed hitbox".into()),
            status(hit),
            hit.hitbox.behavior,
            hit.hitbox.bounds
        ));
    }
    result
}

pub(super) fn render(
    point: Point<Pixels>,
    hits: &[InspectorHitbox],
    picking: bool,
    cx: &mut Context<Inspector>,
) -> Div {
    div().mb_3().p_3().rounded(px(8.)).bg(rgb(theme::SURFACE)).border_1().border_color(rgb(theme::BORDER))
        .child(div().flex().justify_between().mb_2()
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child("Pointer probe"))
            .child(div().text_color(rgb(theme::ACCENT)).child(format!("{:.0}, {:.0}", f32::from(point.x), f32::from(point.y)))))
        .child(div().text_size(px(11.)).text_color(rgb(theme::MUTED)).mb_2().child(
            if picking { "Picking adds hitboxes to otherwise non-interactive elements. Stop picking to inspect normal input." }
            else { "Last application pointer position, or selection center before moving. Front to back; click a row to inspect." }))
        .children(hits.iter().take(32).enumerate().map(|(index, hit)| {
            let id = hit.element.clone();
            div().id(("inspector-hit", index)).debug_selector(move || format!("inspector-hit-{index}")).flex().flex_col().py_2().px_2().gap_1().rounded(px(4.))
                .when(id.is_some(), |row| row.cursor_pointer().hover(|style| style.bg(rgb(theme::HOVER))))
                .child(div().flex().justify_between().gap_2()
                    .child(div().flex_1().min_w_0().text_ellipsis().child(id.as_ref().and_then(|id| id.path.global_id.last()).map(|id| id.to_string()).unwrap_or_else(|| "Unattributed hitbox".into())))
                    .child(div().text_size(px(10.)).text_color(rgb(if hit.mouse { theme::ACCENT } else { theme::MUTED })).child(status(hit))))
                .child(div().text_size(px(10.)).text_color(rgb(theme::MUTED)).child(format!("{:?} · {:.0} × {:.0}", hit.hitbox.behavior, f32::from(hit.hitbox.bounds.size.width), f32::from(hit.hitbox.bounds.size.height))))
                .on_click(cx.listener(move |inspector, _, window, cx| {
                    if let Some(id) = &id {
                        inspector.select(id.clone(), window);
                        cx.notify();
                    }
                }))
        }))
        .when(hits.is_empty(), |panel| panel.child(div().py_2().text_color(rgb(theme::MUTED)).child("No hitbox at this point.")))
        .when(hits.len() > 32, |panel| panel.child(div().text_color(rgb(theme::MUTED)).child(format!("{} more in copied report", hits.len() - 32))))
        .child(div().mt_2().text_size(px(10.)).text_color(rgb(theme::MUTED)).child("Geometric eligibility; handlers may still stop propagation. Pointer capture can route input outside this point."))
}
