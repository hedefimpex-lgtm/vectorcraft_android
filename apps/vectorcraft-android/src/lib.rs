//! VectorCraft on Android.
//!
//! Runs the same [`vectorcraft_ui_egui::VectorcraftApp`] as the desktop and web apps through
//! eframe's NativeActivity runner (wgpu: Vulkan where available, else GLES). Build an APK with
//! `cargo apk build -p vectorcraft-android --release`; see `docs/development.md` ("Android build").
//!
//! Differences from the desktop app:
//! - no TCP control server and no system file dialogs: documents are read and written in the
//!   app's own storage, where the preferences, Swatches, Graphic Styles, Libraries and Data
//!   Recovery folders live too;
//! - touch input arrives as egui's pointer events (one finger draws, as a mouse would).

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "android")]
pub use android::android_main;
