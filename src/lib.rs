//! Android shared-library entry point.
//!
//! The desktop binary lives in `src/main.rs`. For Android builds, that same
//! file is compiled as a module here so `android_main` — the symbol that
//! `android-activity`'s native glue looks up with `dlsym` — ends up inside
//! the `cdylib` that `cargo apk` packages.
//!
//! On every other platform this library is empty: the binary target in
//! `src/main.rs` is the real program.

#[cfg(target_os = "android")]
#[path = "main.rs"]
mod app;

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "Rust" fn android_main(android_app: winit::platform::android::activity::AndroidApp) {
    app::run_android(android_app);
}
