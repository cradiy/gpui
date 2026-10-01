use std::{cell::Cell, rc::Rc};

use crate::components::{
    context_menu::{self, ContextMenu, ContextMenuExt, ContextMenuTrigger},
    dropdown::{DropdownState, dropdown},
    popover::{Popover, PopoverState},
};
use gpui::{
    AppContext, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton, Pixels, Point,
    Render, TestAppContext, TransformationMatrix, VisualTestContext, Window, canvas, div, point,
    prelude::*, px, size,
};
use gpui_effects::transform_group;

#[derive(Clone, Copy)]
enum Kind {
    Dropdown,
    Popover,
    Menu,
    Pointer,
}

#[derive(Clone, Default)]
struct Geometry {
    renders: Rc<Cell<usize>>,
    popup: Rc<Cell<Option<Bounds<Pixels>>>>,
    submenu: Rc<Cell<Option<Bounds<Pixels>>>>,
}

fn measure(bounds: Rc<Cell<Option<Bounds<Pixels>>>>) -> impl IntoElement {
    canvas(move |value, _, _| bounds.set(Some(value)), |_, _, _, _| {})
        .absolute()
        .inset_0()
}

struct Controls {
    kind: Kind,
    dropdown: Entity<DropdownState>,
    popover: Entity<PopoverState>,
    clicks: Rc<Cell<usize>>,
    geometry: Geometry,
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.geometry.renders.set(self.geometry.renders.get() + 1);
        let clicks = self.clicks.clone();
        let popover_clicks = self.clicks.clone();
        let geometry = self.geometry.clone();
        let popup = geometry.popup.clone();
        let popover_bounds = popup.clone();
        let button = || div().w(px(100.)).h(px(30.));
        let surface = move || {
            div()
                .relative()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _, _| {
                    clicks.set(clicks.get() + 1)
                })
                .child(measure(popup))
        };
        let trigger = match self.kind {
            Kind::Dropdown => dropdown(&self.dropdown)
                .min_w(px(0.))
                .w(px(120.))
                .h(px(60.))
                .p_0()
                .border_0()
                .trigger(button())
                .menu(surface())
                .into_any_element(),
            Kind::Popover => Popover::new(&self.popover)
                .min_w(px(0.))
                .w(px(120.))
                .h(px(60.))
                .p_0()
                .border_0()
                .trigger(button())
                .content(move |_, _| {
                    let clicks = popover_clicks.clone();
                    div()
                        .relative()
                        .size_full()
                        .on_mouse_down(MouseButton::Left, move |_, _, _| {
                            clicks.set(clicks.get() + 1)
                        })
                        .child(measure(popover_bounds.clone()))
                })
                .into_any_element(),
            Kind::Menu => ContextMenuTrigger::new(button(), move |_, _| menu(geometry.clone()))
                .id("tracked-menu")
                .into_any_element(),
            Kind::Pointer => button()
                .context_menu(move |_, _| menu(geometry.clone()))
                .into_any_element(),
        };
        div().w(px(600.)).h(px(400.)).child(
            div()
                .absolute()
                .left(px(30.))
                .top(px(40.))
                .w(px(100.))
                .child(trigger),
        )
    }
}

#[gpui::test]
fn tracked_menu_zoom_reuses_trigger_content(cx: &mut TestAppContext) {
    let (handle, mut visual, _, geometry) = open(cx, Kind::Menu);
    click(&mut visual, point(px(180.), px(140.)));
    visual.simulate_mouse_move(point(px(740.), px(550.)), None, Modifiers::default());
    draw(&mut visual);
    let initial_renders = geometry.renders.get();
    let initial_size = geometry.popup.get().unwrap().size;
    for step in 0..60 {
        let scale = 1.2 + step as f32 * 0.01;
        handle
            .update(&mut visual.cx, |view, _, cx| {
                view.matrix = matrix(scale, 20., 30.);
                cx.notify();
            })
            .unwrap();
        draw(&mut visual);
        let bounds = geometry.popup.get().unwrap();
        assert!(
            (bounds.left() - px(30. * scale + 20.)).abs() <= px(1.),
            "step {step}: {bounds:?}"
        );
        assert!(
            (bounds.top() - px(70. * scale + 36.)).abs() <= px(1.),
            "step {step}: {bounds:?}"
        );
        assert_eq!(bounds.size, initial_size);
    }
    assert_eq!(
        geometry.renders.get() - initial_renders,
        0,
        "zoom should reuse trigger content while repositioning the menu"
    );
    click(&mut visual, point(px(740.), px(550.)));
    assert!(!is_open(handle, &mut visual));
    handle
        .update(&mut visual.cx, |view, _, cx| {
            view.matrix = matrix(1.5, 70., 50.);
            cx.notify();
        })
        .unwrap();
    draw(&mut visual);
    click(&mut visual, point(px(190.), px(132.5)));
    assert!(is_open(handle, &mut visual));
    assert_eq!(
        geometry.popup.get().unwrap().origin,
        point(px(115.), px(161.))
    );
}

fn menu(geometry: Geometry) -> ContextMenu {
    ContextMenu::new()
        .w(px(120.))
        .p_0()
        .root_surface(move |_, content, _, _| {
            div()
                .relative()
                .child(content)
                .child(measure(geometry.popup.clone()))
        })
        .submenu_surface(move |_, content, _, _| {
            div()
                .relative()
                .child(content)
                .child(measure(geometry.submenu.clone()))
        })
        .action("Action", |_, _| {})
        .submenu("More", |menu| menu.action("Child", |_, _| {}))
}

struct Probe {
    controls: Entity<Controls>,
    matrix: TransformationMatrix,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(transform_group(
                self.controls
                    .clone()
                    .cached(div().w(px(600.)).h(px(400.)).style().clone())
                    .cache_across_transforms(),
                self.matrix,
            ))
            .child(context_menu::layer(cx))
    }
}

fn matrix(scale: f32, x: f32, y: f32) -> TransformationMatrix {
    TransformationMatrix {
        rotation_scale: [[scale, 0.], [0., scale]],
        translation: [x, y],
    }
}

fn open(
    cx: &mut TestAppContext,
    kind: Kind,
) -> (
    gpui::WindowHandle<Probe>,
    VisualTestContext,
    Rc<Cell<usize>>,
    Geometry,
) {
    cx.update(context_menu::init);
    let clicks = Rc::new(Cell::new(0));
    let clicks_for_view = clicks.clone();
    let geometry = Geometry::default();
    let geometry_for_view = geometry.clone();
    let handle = cx.open_window(size(px(800.), px(600.)), move |window, cx| Probe {
        controls: cx.new(|cx| Controls {
            kind,
            dropdown: cx.new(|cx| DropdownState::new(window, cx)),
            popover: cx.new(|cx| PopoverState::new(window, cx)),
            clicks: clicks_for_view,
            geometry: geometry_for_view,
        }),
        matrix: matrix(2., 20., 30.),
    });
    cx.set_subtree_effects_supported(handle.into(), true);
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    draw(&mut visual);
    (handle, visual, clicks, geometry)
}

fn is_open(handle: gpui::WindowHandle<Probe>, visual: &mut VisualTestContext) -> bool {
    handle
        .update(&mut visual.cx, |view, _, cx| {
            let controls = view.controls.read(cx);
            match controls.kind {
                Kind::Dropdown => controls.dropdown.read(cx).is_open(),
                Kind::Popover => controls.popover.read(cx).is_open(),
                _ => context_menu::is_open(cx),
            }
        })
        .unwrap()
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
}
fn click(cx: &mut VisualTestContext, point: Point<gpui::Pixels>) {
    cx.simulate_click(point, Modifiers::default());
    draw(cx);
}

#[gpui::test]
fn transformed_overlays_follow_current_frame_and_keep_normal_size_and_hits(
    cx: &mut TestAppContext,
) {
    for kind in [Kind::Dropdown, Kind::Popover, Kind::Menu] {
        let (handle, mut visual, clicks, geometry) = open(cx, kind);
        click(&mut visual, point(px(180.), px(140.)));
        assert!(is_open(handle, &mut visual));
        let initial = geometry.popup.get().expect("popup opens");
        assert_eq!(initial.origin, point(px(80.), px(176.)));
        assert_eq!(initial.size.width, px(120.));
        handle
            .update(&mut visual.cx, |view, _, cx| {
                view.matrix = matrix(1.5, 70., 50.);
                cx.notify();
            })
            .unwrap();
        draw(&mut visual);
        let moved = geometry.popup.get().unwrap();
        assert_eq!(moved.origin, point(px(115.), px(161.)));
        assert_eq!(moved.size, initial.size);
        if !matches!(kind, Kind::Menu) {
            click(&mut visual, moved.center());
            assert_eq!(clicks.get(), 1);
            assert!(is_open(handle, &mut visual));
        }
        click(&mut visual, point(px(740.), px(550.)));
        assert!(!is_open(handle, &mut visual), "outside click dismisses");
    }
}

#[gpui::test]
fn transformed_dropdown_trigger_toggles_without_reopening(cx: &mut TestAppContext) {
    let (handle, mut visual, _, _) = open(cx, Kind::Dropdown);
    let point = point(px(180.), px(140.));
    click(&mut visual, point);
    assert!(is_open(handle, &mut visual));
    click(&mut visual, point);
    assert!(!is_open(handle, &mut visual));
}

#[gpui::test]
fn pointer_menu_uses_displayed_click_and_does_not_follow_later_transforms(cx: &mut TestAppContext) {
    let (handle, mut visual, _, geometry) = open(cx, Kind::Pointer);
    visual.simulate_mouse_down(
        point(px(180.), px(140.)),
        MouseButton::Right,
        Modifiers::default(),
    );
    draw(&mut visual);
    let initial = geometry.popup.get().unwrap();
    assert_eq!(initial.origin, point(px(184.), px(144.)));
    handle
        .update(&mut visual.cx, |view, _, cx| {
            view.matrix = matrix(1.5, 70., 50.);
            cx.notify();
        })
        .unwrap();
    draw(&mut visual);
    assert_eq!(geometry.popup.get(), Some(initial));
}

#[gpui::test]
fn tracked_context_submenu_follows_its_current_parent_row(cx: &mut TestAppContext) {
    let (handle, mut visual, _, geometry) = open(cx, Kind::Menu);
    click(&mut visual, point(px(180.), px(140.)));
    visual.simulate_keystrokes("down right");
    draw(&mut visual);
    let root = geometry.popup.get().unwrap();
    let child = geometry.submenu.get().unwrap();
    handle
        .update(&mut visual.cx, |view, _, cx| {
            view.matrix = matrix(1.5, 70., 50.);
            cx.notify();
        })
        .unwrap();
    draw(&mut visual);
    let moved_root = geometry.popup.get().unwrap();
    let moved_child = geometry.submenu.get().unwrap();
    assert_eq!(
        moved_child.origin - child.origin,
        moved_root.origin - root.origin
    );
    assert_eq!(moved_child.size, child.size);
    click(&mut visual, moved_child.center());
    assert!(!is_open(handle, &mut visual));
}

#[gpui::test]
fn rotated_triggers_anchor_to_displayed_edges_with_unscaled_gap(cx: &mut TestAppContext) {
    for kind in [Kind::Dropdown, Kind::Popover, Kind::Menu] {
        let (handle, mut visual, _, geometry) = open(cx, kind);
        click(&mut visual, point(px(180.), px(140.)));
        handle
            .update(&mut visual.cx, |view, _, cx| {
                view.matrix = TransformationMatrix {
                    rotation_scale: [[0., -1.], [1., 0.]],
                    translation: [250., 0.],
                };
                cx.notify();
            })
            .unwrap();
        draw(&mut visual);
        assert_eq!(
            geometry.popup.get().unwrap().origin,
            point(px(180.), px(136.))
        );
    }
}

#[gpui::test]
fn overlay_collision_fits_after_mapping_to_window(cx: &mut TestAppContext) {
    for kind in [Kind::Dropdown, Kind::Popover, Kind::Menu] {
        let (handle, mut visual, clicks, geometry) = open(cx, kind);
        visual.simulate_resize(size(px(400.), px(300.)));
        draw(&mut visual);
        click(&mut visual, point(px(180.), px(140.)));
        handle
            .update(&mut visual.cx, |view, _, cx| {
                view.matrix = matrix(1.5, 250., 150.);
                cx.notify();
            })
            .unwrap();
        draw(&mut visual);
        let popup = geometry.popup.get().unwrap();
        assert!(popup.left() >= px(0.) && popup.right() <= px(400.));
        assert!(popup.top() >= px(0.) && popup.bottom() <= px(300.));
        if !matches!(kind, Kind::Menu) {
            click(&mut visual, popup.center());
            assert_eq!(clicks.get(), 1);
            assert!(is_open(handle, &mut visual));
        }
    }
}
