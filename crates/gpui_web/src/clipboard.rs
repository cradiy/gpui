use gpui::{ClipboardItem, ForegroundExecutor, Task};
use std::cell::{Cell, RefCell};
use wasm_bindgen::JsValue;

const METADATA: &str = "application/x-gpui-metadata";
thread_local! {
    static CACHE: RefCell<Option<ClipboardItem>> = const { RefCell::new(None) };
    static EVENT_DATA: RefCell<Option<web_sys::DataTransfer>> = const { RefCell::new(None) };
    static REVISION: Cell<u64> = const { Cell::new(0) };
}
pub(crate) fn read() -> Option<ClipboardItem> {
    CACHE.with(|cache| cache.borrow().clone())
}
fn cache(item: Option<ClipboardItem>) {
    REVISION.with(|revision| revision.set(revision.get().wrapping_add(1)));
    CACHE.with(|cache| *cache.borrow_mut() = item);
}
fn error(value: JsValue) -> anyhow::Error {
    anyhow::anyhow!("browser clipboard: {value:?}")
}
fn clipboard() -> anyhow::Result<web_sys::Clipboard> {
    web_sys::window()
        .map(|window| window.navigator().clipboard())
        .filter(|clipboard| !wasm_bindgen::JsValue::from(clipboard.clone()).is_undefined())
        .ok_or_else(|| anyhow::anyhow!("browser clipboard requires a secure context"))
}
pub(crate) fn read_async(
    executor: &ForegroundExecutor,
) -> Task<anyhow::Result<Option<ClipboardItem>>> {
    let promise = clipboard().map(|clipboard| clipboard.read_text());
    let revision = REVISION.with(Cell::get);
    executor.spawn(async move {
        let text = wasm_bindgen_futures::JsFuture::from(promise?)
            .await
            .map_err(error)?;
        let item = text.as_string().map(ClipboardItem::new_string);
        // A pending permission prompt must not overwrite a later copy/paste.
        if REVISION.with(Cell::get) == revision {
            cache(item.clone());
        }
        Ok(item)
    })
}
pub(crate) fn write_async(
    item: ClipboardItem,
    executor: &ForegroundExecutor,
) -> Task<anyhow::Result<()>> {
    let Some(text) = item.text() else {
        return Task::ready(Err(anyhow::anyhow!(
            "browser clipboard writing currently supports text"
        )));
    };
    cache(Some(item));
    let promise = clipboard().map(|clipboard| clipboard.write_text(&text));
    executor.spawn(async move {
        wasm_bindgen_futures::JsFuture::from(promise?)
            .await
            .map_err(error)?;
        Ok(())
    })
}
pub(crate) fn write(item: ClipboardItem, executor: &ForegroundExecutor) {
    let handled = EVENT_DATA.with(|event| {
        let event = event.borrow();
        let Some(data) = event.as_ref() else {
            return false;
        };
        if let Some(text) = item.text() {
            let _ = data.set_data("text/plain", &text);
        }
        if let Some(metadata) = item.metadata() {
            let _ = data.set_data(METADATA, metadata);
        }
        cache(Some(item.clone()));
        true
    });
    if !handled {
        let task = write_async(item, executor);
        executor
            .spawn(async move {
                if let Err(error) = task.await {
                    log::warn!("{error:#}");
                }
            })
            .detach();
    }
}

pub(crate) fn with_event(data: web_sys::DataTransfer, paste: bool, dispatch: impl FnOnce()) {
    if paste {
        let text = data.get_data("text/plain").unwrap_or_default();
        let metadata = data.get_data(METADATA).unwrap_or_default();
        cache(Some(if metadata.is_empty() {
            ClipboardItem::new_string(text)
        } else {
            ClipboardItem::new_string_with_metadata(text, metadata)
        }));
    }
    EVENT_DATA.with(|event| *event.borrow_mut() = Some(data));
    dispatch();
    EVENT_DATA.with(|event| event.borrow_mut().take());
}
