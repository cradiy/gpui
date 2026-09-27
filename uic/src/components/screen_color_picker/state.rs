use gpui::{Context, EventEmitter, Rgba, SharedString, Task};

/// The result of a platform screen-color sampling session.
#[derive(Clone, Debug, PartialEq)]
pub enum ScreenColorPickerEvent {
    Picked(Rgba),
    Cancelled,
    Failed(SharedString),
}

/// Shared state for the platform sampling button and caller-owned actions.
pub struct ScreenColorPickerState {
    busy: bool,
    error: Option<SharedString>,
    task: Option<Task<()>>,
}

impl EventEmitter<ScreenColorPickerEvent> for ScreenColorPickerState {}

impl ScreenColorPickerState {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self {
            busy: false,
            error: None,
            task: None,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }
    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Opens the platform sampler. Returns false while a request is pending.
    /// The active sampling session handles cancellation and keyboard interaction.
    pub fn pick(&mut self, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        self.busy = true;
        self.error = None;
        let response = cx.pick_screen_color();
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = response.await.unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "System color picker closed without a result"
                ))
            });
            let _ = this.update(cx, |this, cx| this.complete(result, cx));
        }));
        cx.notify();
        true
    }

    fn complete(&mut self, result: anyhow::Result<Option<Rgba>>, cx: &mut Context<Self>) {
        self.busy = false;
        self.error = None;
        match result {
            Ok(Some(color)) => cx.emit(ScreenColorPickerEvent::Picked(color)),
            Ok(None) => cx.emit(ScreenColorPickerEvent::Cancelled),
            Err(error) => {
                let message: SharedString = format!("{error:#}").into();
                self.error = Some(message.clone());
                cx.emit(ScreenColorPickerEvent::Failed(message));
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, TestAppContext};
    use std::{cell::RefCell, rc::Rc};

    #[gpui::test]
    fn results_distinguish_cancellation_errors_and_selection_and_allow_retry(
        cx: &mut TestAppContext,
    ) {
        let state = cx.new(ScreenColorPickerState::new);
        let events = Rc::new(RefCell::new(Vec::new()));
        let observed = events.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&state, move |_, event: &ScreenColorPickerEvent, _| {
                observed.borrow_mut().push(event.clone())
            })
        });
        cx.update(|cx| {
            state.update(cx, |s, cx| {
                s.busy = true;
                assert!(!s.pick(cx));
                s.complete(Ok(None), cx);
                assert!(!s.is_busy());
                assert!(s.error().is_none());
                s.busy = true;
                s.complete(Err(anyhow::anyhow!("unavailable")), cx);
                assert!(!s.is_busy());
                assert!(s.error().is_some());
                s.busy = true;
                s.complete(Ok(Some(gpui::rgb(0x123456))), cx);
                assert!(!s.is_busy());
                assert!(s.error().is_none());
            })
        });
        assert_eq!(
            *events.borrow(),
            [
                ScreenColorPickerEvent::Cancelled,
                ScreenColorPickerEvent::Failed("unavailable".into()),
                ScreenColorPickerEvent::Picked(gpui::rgb(0x123456))
            ]
        );
    }

    #[gpui::test]
    fn unsupported_platform_finishes_as_error_instead_of_selection(cx: &mut TestAppContext) {
        let state = cx.new(ScreenColorPickerState::new);
        cx.update(|cx| {
            state.update(cx, |s, cx| {
                assert!(s.pick(cx));
                assert!(!s.pick(cx));
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(!state.read(cx).is_busy());
            assert!(state.read(cx).error().is_some());
        });
    }
}
