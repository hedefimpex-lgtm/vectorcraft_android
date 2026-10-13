//! The phone's shared storage (Documents, Download, Pictures…): whether the app may read and write
//! it, and asking the user for that, through the activity's Java side (JNI).
//!
//! Android 11 (API 30) and later: "All files access" (`MANAGE_EXTERNAL_STORAGE`), granted on a
//! settings page the app opens. Android 9 and 10: the storage permissions, asked for in a system
//! dialog. Either way the answer comes when the user comes back to the app, which then checks again.

use std::path::PathBuf;

use jni::JNIEnv;
use jni::objects::{JObject, JValue};
use winit::platform::android::activity::AndroidApp;

/// Where the phone's shared storage starts.
pub const ROOT: &str = "/storage/emulated/0";

/// The storage folders offered in the file dialog: the app's own folder in Documents first.
pub fn places(app_files: Option<PathBuf>) -> Vec<vectorcraft_ui_egui::file_browser::Place> {
    use vectorcraft_ui_egui::file_browser::Place;
    let root = PathBuf::from(ROOT);
    let mut places = vec![];
    // Made once access is granted ([`granted`]).
    places.push(Place { label: "VectorCraft".into(), path: our_folder() });
    for name in ["Documents", "Download", "Pictures"] {
        places.push(Place { label: name.into(), path: root.join(name) });
    }
    places.push(Place { label: "Storage".into(), path: root });
    // The app's own folder on shared storage needs no permission: always readable.
    if let Some(dir) = app_files {
        places.push(Place { label: "App".into(), path: dir });
    }
    places
}

/// The app's own folder in Documents, where documents are saved by default.
fn our_folder() -> PathBuf {
    PathBuf::from(ROOT).join("Documents").join("VectorCraft")
}

/// Run `f` with the JVM's environment for this thread and the activity.
fn with_activity<R>(app: &AndroidApp, f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<R>) -> Option<R> {
    // SAFETY: android-activity keeps the process's JavaVM pointer valid for the app's life.
    #[allow(unsafe_code)]
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    // SAFETY: a global reference to the activity android-activity holds while the app runs; the
    // `JObject` doesn't own it, so dropping it deletes nothing.
    #[allow(unsafe_code)]
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let r = f(&mut env, &activity);
    if r.is_err() {
        // A Java exception left pending would fail every JNI call after it.
        let _ = env.exception_clear();
    }
    r.map_err(|e| log::warn!("storage access: {e}")).ok()
}

/// The device's Android API level.
fn sdk(env: &mut JNIEnv) -> jni::errors::Result<i32> {
    env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()
}

/// Whether the app may read and write all of shared storage now. Once it may, its own folder in
/// Documents is made.
pub fn granted(app: &AndroidApp) -> bool {
    let granted = with_activity(app, |env, activity| {
        if sdk(env)? >= 30 {
            env.call_static_method("android/os/Environment", "isExternalStorageManager", "()Z", &[])?.z()
        } else {
            let perm = env.new_string("android.permission.WRITE_EXTERNAL_STORAGE")?;
            let r = env.call_method(activity, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&perm)])?.i()?;
            // PackageManager.PERMISSION_GRANTED
            Ok(r == 0)
        }
    })
    .unwrap_or(false);
    if granted {
        // Best effort: the folder is offered either way, and says why it can't be listed.
        let _ = std::fs::create_dir_all(our_folder());
    }
    granted
}

/// Ask the user for access to shared storage.
pub fn request(app: &AndroidApp) {
    let asked = with_activity(app, |env, activity| {
        if sdk(env)? >= 30 {
            let package = env.call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
            let package: String = env.get_string(&package.into())?.into();
            let uri = env.new_string(format!("package:{package}"))?;
            let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&uri)])?.l()?;
            let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
            let intent =
                env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[JValue::Object(&action), JValue::Object(&uri)])?;
            if env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)]).is_err() {
                // Some devices have no per-app page: the list of apps instead.
                let _ = env.exception_clear();
                let action = env.new_string("android.settings.MANAGE_ALL_FILES_ACCESS_PERMISSION")?;
                let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;)V", &[JValue::Object(&action)])?;
                env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[JValue::Object(&intent)])?;
            }
        } else {
            let perms = env.new_object_array(2, "java/lang/String", JObject::null())?;
            for (i, p) in ["android.permission.READ_EXTERNAL_STORAGE", "android.permission.WRITE_EXTERNAL_STORAGE"].into_iter().enumerate() {
                let s = env.new_string(p)?;
                env.set_object_array_element(&perms, i as i32, s)?;
            }
            env.call_method(activity, "requestPermissions", "([Ljava/lang/String;I)V", &[JValue::Object(&perms), JValue::Int(1)])?;
        }
        Ok(())
    });
    if asked.is_none() {
        log::warn!("couldn't ask for storage access");
    }
}
