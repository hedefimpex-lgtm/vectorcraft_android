//! The Android shell: logging to logcat, the app's storage folders, the in-window file dialog,
//! the Back button, and the eframe runner.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_ui_egui::file_browser::{Access, FileBrowser};
use vectorcraft_ui_egui::{FilePick, Services, VectorcraftApp};
use winit::platform::android::activity::AndroidApp;

use crate::storage;

/// The entry point the NativeActivity calls (through android-activity's glue) when the app starts.
// android-activity looks the entry point up by this exact name: it can't be mangled.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub fn android_main(android: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("vectorcraft"));
    let options = eframe::NativeOptions { android_app: Some(android.clone()), renderer: eframe::Renderer::Wgpu, ..Default::default() };
    // A panic that ends the activity is caught here and logged: never a crash dialog.
    let outcome = vectorcraft_engine::guard::catch_panic(|| {
        eframe::run_native(
            "VectorCraft",
            options,
            Box::new(move |cc| {
                // Touch screens are read at arm's length: a larger interface than the desktop's.
                cc.egui_ctx.set_zoom_factor(1.25);
                Ok(Box::new(Shell::new(android, cc.egui_ctx.clone())))
            }),
        )
    });
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::error!("VectorCraft couldn't start: {e}"),
        Err(e) => log::error!("VectorCraft stopped after an internal error: {e}"),
    }
}

/// The app, its file dialog, and where it keeps its settings.
struct Shell {
    app: VectorcraftApp,
    browser: Rc<RefCell<FileBrowser>>,
    /// `ui.json` in the app's internal storage, if Android gave it one.
    prefs: Option<PathBuf>,
    /// The window had the focus last frame: losing it (another app comes up) saves the settings,
    /// since Android may end a backgrounded app without telling it.
    focused: bool,
}

impl Shell {
    fn new(android: AndroidApp, ctx: egui::Context) -> Self {
        let data = android.internal_data_path();
        let mut browser = FileBrowser::new(storage::places(android.external_data_path()));
        let (a, b) = (android.clone(), android);
        browser.access = Some(Access { granted: Box::new(move || storage::granted(&a)), request: Box::new(move || storage::request(&b)) });
        let browser = Rc::new(RefCell::new(browser));
        let mut app = VectorcraftApp::new(Session::new(), services(&browser, ctx));
        let prefs = data.as_ref().map(|d| d.join("ui.json"));
        let saved = prefs.as_deref().and_then(read_prefs);
        // First run, or settings from before the touch layout: start in it.
        let touch = saved.as_ref().is_none_or(|(_, has_touch)| !has_touch);
        if let Some((saved, _)) = saved {
            app.ui = saved.sanitized();
        }
        if touch && let Err(e) = app.run("view.touchLayout", serde_json::json!({ "on": true })) {
            log::warn!("touch layout: {e}");
        }
        vectorcraft_ui_egui::prefs_dialog::restore(&mut app);
        if let Some(dir) = &data {
            let folder = |name: &str| Some(dir.join(name).to_string_lossy().to_string());
            app.session.swatch_libraries.set_user_dir(folder("Swatches"));
            app.session.style_libraries.set_user_dir(folder("Graphic Styles"));
            app.session.libraries.set_dir(folder("Libraries"));
            app.session.recovery.set_default_folder(folder("Data Recovery"));
        }
        Self { app, browser, prefs, focused: true }
    }

    fn save_prefs(&self) {
        let Some(p) = &self.prefs else { return };
        let mut ui = self.app.ui.clone();
        ui.engine_prefs = self.app.session.prefs.to_json();
        if let Ok(bytes) = serde_json::to_vec_pretty(&ui) {
            // Preferences are best effort: a failed write keeps the previous file.
            let _ = fileio::write_atomic(p, &bytes);
        }
    }
}

/// The saved settings, and whether they say whether the touch layout is on.
fn read_prefs(path: &Path) -> Option<(vectorcraft_ui_egui::UiState, bool)> {
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let has_touch = json.get("touch_layout").is_some();
    Some((serde_json::from_value(json).ok()?, has_touch))
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let focused = ctx.input(|i| i.focused);
        if self.focused && !focused {
            self.save_prefs();
        }
        self.focused = focused;
        self.app.logic(ctx);
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Back closes the file dialog, else acts as Escape (closes a menu or a dialog, ends what
        // the tool is drawing). It never leaves the app: unsaved work would be at stake.
        let back = |e: &egui::Event| matches!(e, egui::Event::Key { key: egui::Key::BrowserBack, .. });
        let pressed = raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::BrowserBack, pressed: true, .. }));
        let mut browser = self.browser.borrow_mut();
        if browser.is_open() {
            raw.events.retain(|e| !back(e));
            if pressed {
                browser.cancel();
            }
        } else {
            for e in &mut raw.events {
                if let egui::Event::Key { key, physical_key, .. } = e
                    && *key == egui::Key::BrowserBack
                {
                    *key = egui::Key::Escape;
                    *physical_key = Some(egui::Key::Escape);
                }
            }
        }
        drop(browser);
        self.app.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
        self.browser.borrow_mut().show(ui.ctx());
    }
    fn on_exit(&mut self) {
        self.save_prefs();
    }
}

/// Files are read and written by path (shared storage once access is granted); the file dialogs
/// are the in-window [`FileBrowser`], answering like a system dialog.
fn services(browser: &Rc<RefCell<FileBrowser>>, ctx: egui::Context) -> Services {
    // Never shown in line: `start_pick` always opens the in-window dialog. They say which kinds of
    // dialog there are.
    let none = |_: &FilePick| None;
    Services {
        pick_open: Some(Box::new(none)),
        pick_save: Some(Box::new(none)),
        pick_open_multi: Some(Box::new(Vec::new)),
        pick_folder: Some(Box::new(|| None)),
        start_pick: Some(FileBrowser::starter(browser, ctx)),
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        write: Some(Box::new(write_file)),
        write_shared: Some(std::sync::Arc::new(write_file)),
        stream_shared: Some(std::sync::Arc::new(|p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
            fileio::write_file_with(p, fill).map_err(|e| e.to_string())
        })),
        ..Default::default()
    }
}

fn write_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    fileio::write_atomic(Path::new(path), bytes).map_err(|e| e.to_string())
}
