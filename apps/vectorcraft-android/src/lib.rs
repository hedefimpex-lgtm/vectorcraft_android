//! VectorCraft on Android.
//!
//! Runs the same [`vectorcraft_ui_egui::VectorcraftApp`] as the desktop and web apps through
//! eframe's NativeActivity runner (wgpu: Vulkan where available, else GLES). Build an APK with
//! `cargo apk build -p vectorcraft-android --release`; see `docs/development.md` ("Android build").
//!
//! Differences from the desktop app:
//! - no TCP control server;
//! - the file dialogs are drawn in the window (`vectorcraft_ui_egui::file_browser`), over the
//!   phone's shared storage once the user allows access to all files (`storage`);
//! - it starts in View › Touch Layout (`vectorcraft_ui_egui::touch_layout`): big buttons along
//!   the top, the tools along the bottom;
//! - Back closes the file dialog, a menu or a dialog (as Escape), never the app;
//! - the settings are saved whenever the app leaves the screen, as Android may end it then.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod storage;

#[cfg(target_os = "android")]
pub use android::android_main;
