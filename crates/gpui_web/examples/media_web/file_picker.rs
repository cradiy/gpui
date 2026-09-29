use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{File, HtmlInputElement, Url};

pub struct FilePicker {
    input: HtmlInputElement,
    changed: Closure<dyn FnMut(web_sys::Event)>,
}

impl FilePicker {
    pub fn new(kind: &str, mut selected: impl FnMut(File) + 'static) -> Result<Self, JsValue> {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| JsValue::from_str("No browser document"))?;
        let input: HtmlInputElement = document.create_element("input")?.dyn_into()?;
        input.set_type("file");
        input.set_accept(&format!("{kind}/*"));
        input.set_attribute("hidden", "")?;
        input.set_attribute("data-media-picker", kind)?;
        let source = input.clone();
        let changed = Closure::wrap(Box::new(move |_: web_sys::Event| {
            if let Some(file) = source.files().and_then(|files| files.get(0)) {
                selected(file);
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        let picker = Self { input, changed };
        picker
            .input
            .add_event_listener_with_callback("change", picker.changed.as_ref().unchecked_ref())?;
        document
            .body()
            .ok_or_else(|| JsValue::from_str("No document body"))?
            .append_child(&picker.input)?;
        Ok(picker)
    }

    pub fn open(&self) {
        // Allow selecting the same file again after playback or a decode error.
        self.input.set_value("");
        self.input.click();
    }
}

impl Drop for FilePicker {
    fn drop(&mut self) {
        let _ = self
            .input
            .remove_event_listener_with_callback("change", self.changed.as_ref().unchecked_ref());
        self.input.remove();
    }
}

pub struct ObjectUrl(String);

impl ObjectUrl {
    pub fn new(file: &File) -> Result<Self, JsValue> {
        Url::create_object_url_with_blob(file).map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for ObjectUrl {
    fn drop(&mut self) {
        let _ = Url::revoke_object_url(&self.0);
    }
}
