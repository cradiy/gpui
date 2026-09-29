use futures::{
    channel::oneshot,
    future::{Either, LocalBoxFuture, select},
};
use gpui::{FilePromptOptions, PlatformFile, SelectedFile};
use gpui_util::browser::BrowserResource;
use std::{cell::RefCell, rc::Rc, sync::Arc};
use wasm_bindgen::prelude::*;

fn error(value: impl std::fmt::Debug) -> anyhow::Error {
    anyhow::anyhow!("browser file picker: {value:?}")
}

#[derive(Debug)]
struct FileResource {
    file: web_sys::File,
    url: String,
}
impl Drop for FileResource {
    fn drop(&mut self) {
        let _ = web_sys::Url::revoke_object_url(&self.url);
    }
}
#[derive(Debug)]
struct BrowserFile {
    name: String,
    url: String,
    resource: BrowserResource<FileResource>,
}
impl PlatformFile for BrowserFile {
    fn name(&self) -> &str {
        &self.name
    }
    fn url(&self) -> Option<&str> {
        Some(&self.url)
    }
    fn read(&self) -> LocalBoxFuture<'static, anyhow::Result<Vec<u8>>> {
        let resource = self.resource.clone();
        Box::pin(async move {
            let promise = resource.with(|r| r.file.array_buffer()).ok_or_else(|| {
                anyhow::anyhow!("browser files must be read on their owning thread")
            })?;
            let buffer = wasm_bindgen_futures::JsFuture::from(promise)
                .await
                .map_err(error)?;
            Ok(js_sys::Uint8Array::new(&buffer).to_vec())
        })
    }
}
fn selected_file(file: web_sys::File) -> anyhow::Result<SelectedFile> {
    let name = file.name();
    let url = web_sys::Url::create_object_url_with_blob(&file).map_err(error)?;
    Ok(SelectedFile::new(Arc::new(BrowserFile {
        name,
        url: url.clone(),
        resource: BrowserResource::new(FileResource { file, url }),
    })))
}

struct Picker {
    input: web_sys::HtmlInputElement,
    listeners: Vec<(&'static str, Closure<dyn FnMut(web_sys::Event)>)>,
}
impl Drop for Picker {
    fn drop(&mut self) {
        for (name, listener) in &self.listeners {
            let _ = self
                .input
                .remove_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
        }
        self.input.remove();
    }
}

pub(crate) fn prompt(
    options: FilePromptOptions,
) -> oneshot::Receiver<anyhow::Result<Option<Vec<SelectedFile>>>> {
    let (mut tx, rx) = oneshot::channel();
    let setup = || -> anyhow::Result<_> {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .ok_or_else(|| anyhow::anyhow!("no browser document"))?;
        let input: web_sys::HtmlInputElement = document
            .create_element("input")
            .map_err(error)?
            .dyn_into()
            .map_err(error)?;
        input.set_type("file");
        input.set_multiple(options.multiple);
        input.set_hidden(true);
        document
            .body()
            .ok_or_else(|| anyhow::anyhow!("no document body"))?
            .append_child(&input)
            .map_err(error)?;
        let mut picker = Picker {
            input: input.clone(),
            listeners: Vec::new(),
        };
        let (done, result) = oneshot::channel();
        let done = Rc::new(RefCell::new(Some(done)));
        for name in ["change", "cancel"] {
            let done = done.clone();
            let input = input.clone();
            let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                let Some(tx) = done.borrow_mut().take() else {
                    return;
                };
                let files = if name == "cancel" {
                    Ok(None)
                } else {
                    let files = input.files();
                    let files = files
                        .into_iter()
                        .flat_map(|files| (0..files.length()).filter_map(move |i| files.get(i)))
                        .map(selected_file)
                        .collect::<anyhow::Result<Vec<_>>>();
                    files.map(|files| if files.is_empty() { None } else { Some(files) })
                };
                let _ = tx.send(files);
            });
            picker
                .input
                .add_event_listener_with_callback(name, listener.as_ref().unchecked_ref())
                .map_err(error)?;
            picker.listeners.push((name, listener));
        }
        input.show_picker().map_err(error)?;
        Ok((picker, result))
    };
    match setup() {
        Err(error) => {
            let _ = tx.send(Err(error));
        }
        Ok((picker, result)) => wasm_bindgen_futures::spawn_local(async move {
            let result = {
                match select(Box::pin(result), Box::pin(tx.cancellation())).await {
                    Either::Left((result, _)) => Some(result),
                    Either::Right(_) => None,
                }
            };
            drop(picker);
            if let Some(result) = result {
                let _ = tx.send(result.unwrap_or_else(|e| Err(e.into())));
            }
        }),
    }
    rx
}
