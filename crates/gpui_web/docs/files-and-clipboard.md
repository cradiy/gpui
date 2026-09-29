# Files and clipboard

Call `cx.prompt_for_files(FilePromptOptions { multiple: true })` from a click
handler. Await the returned receiver: cancellation yields `Ok(None)`, and a
selection yields `Ok(Some(files))`. The same API works on desktop and Web.

Each `SelectedFile` exposes its name and asynchronous `read()` method. Desktop
files also expose `path()`; browser files expose `url()`. Contents are read only
when requested. Retain the file handle while using its URL for images or media.
Dropping the final handle releases the URL. `prompt_for_paths` and save-path
prompts remain native-only: browsers do not expose filesystem paths.

Use `read_from_clipboard_async()` and `write_to_clipboard_async()` from a user
gesture for text clipboard buttons. They report browser permission errors and
require a secure context (HTTPS or localhost). Browser permission prompts differ.

Keyboard copy, cut and paste use the existing synchronous clipboard methods
inside browser clipboard events. Text metadata is retained for GPUI clipboard
events. Outside an event, synchronous reads return the last local clipboard
snapshot; use the asynchronous method to refresh it from the system. Async text
reads carry no GPUI metadata. Image and file clipboard entries are not supported.

The `media_web` example selects audio/video files, displays an independently
extracted first frame and provides text clipboard buttons. Video imports should
await `VideoFrameExtractor::initial_frame()` and retain the selected file;
blocking extraction and `std::fs` access are unavailable in browsers.
