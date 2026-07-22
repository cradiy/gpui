use gpui::{
    AnyElement, Div, InteractiveElement, Interactivity, IntoElement, ParentElement,
    StyleRefinement, Styled, div,
};

/// Creates a container that layers application UI over a video element.
pub fn video_container(video: impl IntoElement) -> VideoContainer {
    VideoContainer::new(video)
}

/// A transparent video container whose children share its complete bounds.
///
/// The supplied video element is painted first. Application-owned children are
/// then painted over the same container, including any letterbox area. Passing
/// an `Entity<VideoPlayer>` keeps video updates independent from the overlay's
/// render lifecycle.
pub struct VideoContainer {
    element: Div,
}

impl VideoContainer {
    pub fn new(video: impl IntoElement) -> Self {
        Self {
            element: div()
                .relative()
                .size_full()
                .overflow_hidden()
                .child(div().absolute().inset_0().child(video)),
        }
    }
}

impl ParentElement for VideoContainer {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}

impl Styled for VideoContainer {
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl InteractiveElement for VideoContainer {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}

impl IntoElement for VideoContainer {
    type Element = Div;

    fn into_element(self) -> Self::Element {
        self.element
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{
        AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Styled,
        TestAppContext, Window, canvas, div, point, px, size,
    };

    use super::video_container;

    struct EmptyVideo;

    impl Render for EmptyVideo {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }

    struct OverlayBoundsView {
        video: Entity<EmptyVideo>,
        overlay_bounds: Rc<Cell<Bounds<gpui::Pixels>>>,
    }

    impl Render for OverlayBoundsView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let overlay_bounds = self.overlay_bounds.clone();
            video_container(self.video.clone()).child(
                canvas(
                    move |bounds, _, _| overlay_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .size_full(),
            )
        }
    }

    #[gpui::test]
    fn overlay_layout_matches_complete_video_container(cx: &mut TestAppContext) {
        let overlay_bounds = Rc::new(Cell::new(Bounds::default()));
        let window = cx.open_window(size(px(1000.0), px(500.0)), {
            let overlay_bounds = overlay_bounds.clone();
            move |_, cx| OverlayBoundsView {
                video: cx.new(|_| EmptyVideo),
                overlay_bounds,
            }
        });

        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear();
        })
        .unwrap();

        assert_eq!(
            overlay_bounds.get(),
            Bounds::new(point(px(0.0), px(0.0)), size(px(1000.0), px(500.0)))
        );
    }
}
