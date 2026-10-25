//! The browser shell: web `Services`, drag-and-drop, and the eframe web runner.

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_engine::Session;
use deckcraft_ui_egui::{Inbox, InboxEvent, InboxFile, Services, SlideApp};
use wasm_bindgen::JsCast as _;

const PRESENTATION_EXTS: &[&str] = &["deckcraft", "pptx", "potx", "ppsx", "txt", "md"];
const PICTURE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];
const AUDIO_EXTS: &[&str] = &["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aif", "aiff"];
const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mov", "webm"];
const CANVAS_ID: &str = "deckcraft_canvas";
const LOADING_ID: &str = "deckcraft_loading";

pub fn start() {
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let mut options = eframe::WebOptions::default();
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let page_query = query();
        let nextcloud = Rc::new(RefCell::new(crate::nextcloud::path_from_query(&page_query)));
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    if let Some(rs) = &cc.wgpu_render_state {
                        log::info!("deckcraft-web {}: wgpu backend {:?}", env!("CARGO_PKG_VERSION"), rs.adapter.get_info().backend);
                    }
                    let inbox: Inbox = Inbox::default();
                    let mut app = SlideApp::new(Session::new(), services(inbox.clone(), cc.egui_ctx.clone(), nextcloud.clone()));
                    app.nextcloud_path = nextcloud.borrow().clone();
                    let q = page_query.clone();
                    let has_nc = nextcloud.borrow().is_some();
                    if has_nc {
                        // A Nextcloud path starts with an empty presentation; the file is loaded
                        // asynchronously and, if missing (404), left empty so Save creates it.
                        if let Err(e) = app.run("file.new", serde_json::json!({})) {
                            log::error!("new presentation: {e}");
                        }
                        if let Some(path) = nextcloud.borrow().as_deref() {
                            open_from_nextcloud(path.to_string(), inbox.clone(), cc.egui_ctx.clone());
                        }
                    } else if !q.contains("blank") {
                        if let Err(e) = deckcraft_engine::sample::open_sample(&mut app.session) {
                            log::error!("sample deck: {e}");
                        }
                    } else if let Err(e) = app.run("file.new", serde_json::json!({})) {
                        log::error!("new presentation: {e}");
                    }
                    if q.contains("show") {
                        app.start_show(0, false);
                    }
                    Ok(Box::new(WebShell { app, inbox }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>DeckCraft failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

/// Wraps the app to read dropped files asynchronously (browsers can't read them synchronously)
/// and feed them through the inbox.
struct WebShell {
    app: SlideApp,
    inbox: Inbox,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            // Capture the document and slide now. The bytes arrive later, after the user may
            // have switched slides.
            let id = self.app.begin_external_file("drop");
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        push_inbox(&inbox, InboxEvent::File(InboxFile { id, name, bytes }));
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        log::error!("couldn't read dropped file {name}: {e}");
                        push_inbox(&inbox, InboxEvent::Cancelled { id });
                        ctx.request_repaint();
                    }
                }
            });
        }
        self.app.logic(ctx);
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
}

/// Web services: no file paths, so `pick_open`/`pick_save`/`read` stay `None` and the UI falls
/// back to `open_async` (browser picker → inbox) and `download`.
fn services(inbox: Inbox, ctx: egui::Context, nextcloud: Rc<RefCell<Option<String>>>) -> Services {
    let open_inbox = inbox.clone();
    let _picture_ctx = ctx.clone();
    let write_ctx = ctx.clone();
    let download_ctx = ctx.clone();
    let write_nextcloud = nextcloud.clone();
    let download_nextcloud = nextcloud;
    Services {
        open_async: Some(Box::new(move |purpose: &str, request: u64| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = match purpose {
                "picture" => rfd::AsyncFileDialog::new().add_filter("Pictures", PICTURE_EXTS),
                "audio" => rfd::AsyncFileDialog::new().add_filter("Audio", AUDIO_EXTS),
                "video" => rfd::AsyncFileDialog::new().add_filter("Video", VIDEO_EXTS),
                _ => rfd::AsyncFileDialog::new().add_filter("Presentations", PRESENTATION_EXTS),
            };
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = dialog.pick_file().await else {
                    push_inbox(&inbox, InboxEvent::Cancelled { id: request });
                    ctx.request_repaint();
                    return;
                };
                let bytes = file.read().await;
                push_inbox(&inbox, InboxEvent::File(InboxFile { id: request, name: file.file_name(), bytes }));
                ctx.request_repaint();
            });
        })),
        write: Some(Box::new(move |name: &str, bytes: &[u8]| match crate::nextcloud::save_target(write_nextcloud.borrow().as_deref(), name) {
            Some(target) => save_to_nextcloud(&write_ctx, target, bytes),
            None => download(name, bytes),
        })),
        download: Some(Box::new(move |name: &str, bytes: &[u8]| {
            if let Some(target) = crate::nextcloud::save_target(download_nextcloud.borrow().as_deref(), name) {
                if let Err(e) = save_to_nextcloud(&download_ctx, target, bytes) {
                    log::error!("Nextcloud save failed: {e}");
                }
            } else if let Err(e) = download(name, bytes) {
                log::error!("download of {name} failed: {e}");
            }
        })),
        inbox: Some(inbox),
        ..Default::default()
    }
}

fn push_inbox(inbox: &Inbox, event: InboxEvent) {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(event);
}

/// Load `path` through the Nextcloud passage into the inbox, as if it had been picked.
fn open_from_nextcloud(path: String, inbox: Inbox, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let result: Result<Option<Vec<u8>>, String> = async {
            let window = web_sys::window().ok_or("no window")?;
            let resp: web_sys::Response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(&crate::nextcloud::url_for(&path)))
                .await
                .map_err(|e| format!("{e:?}"))?
                .dyn_into()
                .map_err(|_| "no response".to_string())?;
            let status = resp.status();
            if status == 404 {
                return Ok(None);
            }
            if !resp.ok() {
                return Err(format!("HTTP {status}"));
            }
            let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().map_err(|e| format!("{e:?}"))?).await.map_err(|e| format!("{e:?}"))?;
            Ok(Some(js_sys::Uint8Array::new(&buf).to_vec()))
        }
        .await;
        match result {
            Ok(Some(bytes)) => {
                let name = crate::nextcloud::file_name(&path).to_string();
                push_inbox(&inbox, InboxEvent::File(InboxFile { id: 0, name, bytes }));
            }
            Ok(None) => {
                log::info!("Nextcloud path does not exist yet; starting empty: {path}");
            }
            Err(e) => {
                log::error!("couldn't load {path} from Nextcloud: {e}");
            }
        }
        ctx.request_repaint();
    });
}

/// Write `bytes` to `path` in Nextcloud (HTTP PUT through the passage).
fn save_to_nextcloud(ctx: &egui::Context, path: String, bytes: &[u8]) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let init = web_sys::RequestInit::new();
    init.set_method("PUT");
    init.set_body(&js_sys::Uint8Array::from(bytes).into());
    let prom = window.fetch_with_str_and_init(&crate::nextcloud::url_for(&path), &init);
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let outcome = match wasm_bindgen_futures::JsFuture::from(prom).await.map(|r| r.dyn_into::<web_sys::Response>()) {
            Ok(Ok(resp)) if resp.ok() => log::info!("In Nextcloud gespeichert: {path}"),
            Ok(Ok(resp)) => log::error!("Nextcloud hat das Speichern abgelehnt (HTTP {})", resp.status()),
            Ok(Err(_)) => log::error!("Nextcloud: keine Antwort"),
            Err(e) => log::error!("Nextcloud nicht erreichbar: {e:?}"),
        };
        let _ = outcome;
        ctx.request_repaint();
    });
    Ok(())
}

/// Trigger a browser download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "deckcraft".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(&name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    // Revoke after the click has been dispatched; the download keeps its own reference.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("gif") => "image/gif",
        Some("txt") => "text/plain",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("deckcraft") => "application/zip",
        _ => "application/octet-stream",
    }
}
