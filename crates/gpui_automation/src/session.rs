use crate::{Selector, Snapshot};
use anyhow::{Result, anyhow};
use gpui::{AnyWindowHandle, App, AppContext, AsyncApp, AutomationAction, Pixels, Size};
use std::time::Duration;

/// In-process inspection entry points. Call from GPUI's foreground context.
pub struct Session;

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub handle: AnyWindowHandle,
    pub title: String,
    pub viewport_size: Size<Pixels>,
    pub scale_factor: f32,
    pub active: bool,
}

impl Session {
    /// Activates the uniquely matched node semantically, without pointer hit testing.
    pub fn click(window: AnyWindowHandle, selector: &Selector, cx: &mut App) -> Result<u64> {
        Self::perform(window, selector, AutomationAction::Click, cx)
    }

    pub fn focus(window: AnyWindowHandle, selector: &Selector, cx: &mut App) -> Result<u64> {
        Self::perform(window, selector, AutomationAction::Focus, cx)
    }

    /// Replaces a value through the node's SetValue handler. Does not simulate
    /// typing, clipboard operations, selection or IME composition.
    pub fn set_value(
        window: AnyWindowHandle,
        selector: &Selector,
        value: impl Into<String>,
        cx: &mut App,
    ) -> Result<u64> {
        Self::perform(
            window,
            selector,
            AutomationAction::SetValue(value.into()),
            cx,
        )
    }

    fn perform(
        window: AnyWindowHandle,
        selector: &Selector,
        action: AutomationAction,
        cx: &mut App,
    ) -> Result<u64> {
        cx.update_window(window, |_, window, cx| {
            let snapshot = Snapshot::new(
                window
                    .automation_snapshot()
                    .ok_or_else(|| anyhow!("automation snapshot is not ready"))?,
            );
            let node = snapshot.find(selector).one()?;
            let generation = snapshot.data().generation;
            window.perform_automation_action(generation, node.id, action, cx)?;
            Ok(generation)
        })?
    }

    /// Waits for a completed snapshot satisfying a predicate. Predicate errors,
    /// closed windows and disabled collection fail immediately. The timeout uses
    /// GPUI's scheduler clock; dropping the future cancels polling.
    pub async fn wait_for(
        window: AnyWindowHandle,
        timeout: Duration,
        mut predicate: impl FnMut(&Snapshot) -> Result<bool>,
        cx: &mut AsyncApp,
    ) -> Result<Snapshot> {
        let executor = cx.background_executor().clone();
        let start = executor.now();
        loop {
            let data = cx.update(|cx| {
                cx.update_window(window, |_, window, _| {
                    anyhow::ensure!(
                        window.is_automation_enabled(),
                        "automation collection is disabled"
                    );
                    Ok(window.automation_snapshot())
                })
            })??;
            if let Some(data) = data {
                let snapshot = Snapshot::new(data);
                if predicate(&snapshot)? {
                    return Ok(snapshot);
                }
            }
            let elapsed = executor.now().duration_since(start);
            anyhow::ensure!(
                elapsed < timeout,
                "timed out waiting for automation state after {timeout:?}"
            );
            executor
                .timer((timeout - elapsed).min(Duration::from_millis(16)))
                .await;
        }
    }

    /// Waits for the first snapshot newer than an action's returned generation.
    /// This is not an animation-idle or GPU-presentation guarantee.
    pub async fn wait_for_draw(
        window: AnyWindowHandle,
        after_generation: u64,
        timeout: Duration,
        cx: &mut AsyncApp,
    ) -> Result<Snapshot> {
        Self::wait_for(
            window,
            timeout,
            |snapshot| Ok(snapshot.data().generation > after_generation),
            cx,
        )
        .await
    }

    /// Lists open windows without enabling semantic collection.
    pub fn windows(cx: &mut App) -> Result<Vec<WindowInfo>> {
        cx.windows()
            .into_iter()
            .map(|handle| {
                cx.update_window(handle, |_, window, _| WindowInfo {
                    handle,
                    title: window.window_title(),
                    viewport_size: window.viewport_size(),
                    scale_factor: window.scale_factor(),
                    active: window.is_window_active(),
                })
            })
            .collect()
    }

    pub fn window_by_title(title: &str, cx: &mut App) -> Result<AnyWindowHandle> {
        let windows: Vec<_> = Self::windows(cx)?
            .into_iter()
            .filter(|window| window.title == title)
            .collect();
        match windows.as_slice() {
            [window] => Ok(window.handle),
            [] => Err(anyhow!("no window has title {title:?}")),
            _ => Err(anyhow!("multiple windows have title {title:?}")),
        }
    }

    /// Enables collection and schedules a redraw. Does not synchronously draw.
    pub fn enable(window: AnyWindowHandle, cx: &mut App) -> Result<()> {
        cx.update_window(window, |_, window, _| window.set_automation_enabled(true))?
    }

    pub fn disable(window: AnyWindowHandle, cx: &mut App) -> Result<()> {
        cx.update_window(window, |_, window, _| window.set_automation_enabled(false))?
    }

    /// Reads the latest completed snapshot, failing if collection is not ready.
    pub fn snapshot(window: AnyWindowHandle, cx: &mut App) -> Result<Snapshot> {
        cx.update_window(window, |_, window, _| window.automation_snapshot())?
            .map(Snapshot::new)
            .ok_or_else(|| anyhow!("no automation snapshot; enable collection and wait for a draw"))
    }
}
