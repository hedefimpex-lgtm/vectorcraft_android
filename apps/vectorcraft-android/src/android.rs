//! The Android shell: logging to logcat, the app's storage folder, and the eframe runner.

use std::path::{Path, PathBuf};

use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_ui_egui::{Services, VectorcraftApp};
use winit::platform::android::activity::AndroidApp;

/// The entry point the NativeActivity calls (through android-activity's glue) when the app starts.
// android-activity looks the entry point up by this exact name: it can't be mangled.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub fn android_main(android: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("vectorcraft"));
    let data = android.internal_data_path();
    let options = eframe::NativeOptions { android_app: Some(android), renderer: eframe::Renderer::Wgpu, ..Default::default() };
    // A panic that ends the activity is caught here and logged: never a crash dialog.
    let outcome = vectorcraft_engine::guard::catch_panic(|| {
        eframe::run_native(
            "VectorCraft",
            options,
            Box::new(move |cc| {
                // Touch screens are read at arm's length: a larger interface than the desktop's.
                cc.egui_ctx.set_zoom_factor(1.25);
                Ok(Box::new(Shell::new(data)))
            }),
        )
    });
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::error!("VectorCraft couldn't start: {e}"),
        Err(e) => log::error!("VectorCraft stopped after an internal error: {e}"),
    }
}

/// The app and where it keeps its files.
struct Shell {
    app: VectorcraftApp,
    /// `ui.json` in the app's internal storage, if Android gave it one.
    prefs: Option<PathBuf>,
}

impl Shell {
    fn new(data: Option<PathBuf>) -> Self {
        let mut app = VectorcraftApp::new(Session::new(), services());
        let prefs = data.as_ref().map(|d| d.join("ui.json"));
        if let Some(saved) = prefs.as_deref().and_then(read_prefs) {
            app.ui = saved.sanitized();
        }
        vectorcraft_ui_egui::prefs_dialog::restore(&mut app);
        if let Some(dir) = &data {
            let folder = |name: &str| Some(dir.join(name).to_string_lossy().to_string());
            app.session.swatch_libraries.set_user_dir(folder("Swatches"));
            app.session.style_libraries.set_user_dir(folder("Graphic Styles"));
            app.session.libraries.set_dir(folder("Libraries"));
            app.session.recovery.set_default_folder(folder("Data Recovery"));
        }
        Self { app, prefs }
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

fn read_prefs(path: &Path) -> Option<vectorcraft_ui_egui::UiState> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.app.logic(ctx);
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.app.raw_input_hook(raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.ui(ui);
    }
    fn on_exit(&mut self) {
        self.save_prefs();
    }
}

/// Files are read and written in place; Save and Export write from a worker thread too.
fn services() -> Services {
    Services {
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
