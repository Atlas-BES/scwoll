//! Android-only platform glue (JNI).
//!
//! Android has no `mpv`/`ffmpeg` binaries, so:
//!
//! * **Playback** is delegated to whatever the user has installed for
//!   `video/*` by firing an `ACTION_VIEW` intent (system video player, VLC,
//!   MX Player, the YouTube app for YouTube links, …).
//! * **Thumbnails** for direct video links come from Android's own
//!   `MediaMetadataRetriever` (frames + embedded artwork), which works for
//!   local files and `http(s)://` URLs.
//!
//! Every entry point here attaches the *calling* thread to the JVM, so these
//! are safe to use from the thumbnail worker threads.

use jni::objects::{JObject, JValue};
use jni::{JNIEnv, JavaVM};

/// `android.media.MediaMetadataRetriever.METADATA_KEY_DURATION`
const METADATA_KEY_DURATION: i32 = 9;
/// `MediaMetadataRetriever.OPTION_CLOSEST_SYNC`
const OPTION_CLOSEST_SYNC: i32 = 2;
/// `Intent.FLAG_ACTIVITY_NEW_TASK` — required when starting from a
/// non-Activity context.
const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;
/// `Intent.FLAG_GRANT_READ_URI_PERMISSION`
const FLAG_GRANT_READ_URI_PERMISSION: i32 = 0x0000_0001;

fn java_error(e: jni::errors::Error) -> String {
    format!("JNI error: {e}")
}

fn vm() -> Result<JavaVM, String> {
    let ctx = ndk_context::android_context();
    // SAFETY: `android-activity` initialised the context with a live JavaVM
    // before calling `android_main`, and we only wrap it (never drop it).
    unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|e| format!("no JavaVM: {e}"))
}

/// The current `Activity`, as a (not owned) JNI reference.
fn activity<'a>() -> JObject<'a> {
    let ctx = ndk_context::android_context();
    // SAFETY: a JNI global reference to the activity, valid while the app runs.
    unsafe { JObject::from_raw(ctx.context() as jni::sys::jobject) }
}

/// How the installed apps can handle a `video/*` link.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VideoHandlers {
    /// Nothing handles `video/*` — hand the URL to the browser instead.
    None,
    /// The primary handler is a gallery-style app without INTERNET access, so
    /// it cannot stream a remote URL (it would just close again). Let the user
    /// pick, or fall back to the browser when it is the only option.
    CannotStream { others: bool },
    /// A handler exists that can fetch the URL itself (VLC, MX Player, …).
    Streams,
}

/// Inspects the packages matching `intent` for `android.permission.INTERNET`:
/// a media player that cannot reach the network cannot play an `http(s)` link.
fn video_handlers(env: &mut JNIEnv<'_>, intent: &JObject<'_>) -> VideoHandlers {
    let inspect = |env: &mut JNIEnv<'_>| -> Option<(usize, bool, bool)> {
        let activity = activity();
        let pm = env
            .call_method(
                &activity,
                "getPackageManager",
                "()Landroid/content/pm/PackageManager;",
                &[],
            )
            .ok()?
            .l()
            .ok()?;
        let list = env
            .call_method(
                &pm,
                "queryIntentActivities",
                "(Landroid/content/Intent;I)Ljava/util/List;",
                &[intent.into(), JValue::Int(0)],
            )
            .ok()?
            .l()
            .ok()?;
        if list.is_null() {
            return Some((0, false, false));
        }
        let count = env.call_method(&list, "size", "()I", &[]).ok()?.i().ok()?;
        let perm = env.new_string("android.permission.INTERNET").ok()?;
        let mut any = false;
        let mut first = false;
        for i in 0..count.clamp(0, 24) {
            let Some(info) = env
                .call_method(&list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(i)])
                .ok()
                .and_then(|v| v.l().ok())
                .filter(|o| !o.is_null())
            else {
                continue;
            };
            let Some(app) = env
                .get_field(&info, "activityInfo", "Landroid/content/pm/ActivityInfo;")
                .ok()
                .and_then(|v| v.l().ok())
                .filter(|o| !o.is_null())
            else {
                continue;
            };
            let Some(pkg) = env
                .get_field(&app, "packageName", "Ljava/lang/String;")
                .ok()
                .and_then(|v| v.l().ok())
                .filter(|o| !o.is_null())
            else {
                continue;
            };
            let granted = env
                .call_method(
                    &pm,
                    "checkPermission",
                    "(Ljava/lang/String;Ljava/lang/String;)I",
                    &[(&perm).into(), (&pkg).into()],
                )
                .ok()
                .and_then(|v| v.i().ok())
                .unwrap_or(-1);
            let ok = granted == 0;
            if i == 0 {
                first = ok;
            }
            any |= ok;
        }
        Some((count.max(0) as usize, first, any))
    };

    let Some((count, first_streams, any_streams)) = inspect(env) else {
        // PackageManager inspection failed: trust the plain intent.
        return VideoHandlers::Streams;
    };
    if count == 0 {
        VideoHandlers::None
    } else if first_streams {
        VideoHandlers::Streams
    } else {
        VideoHandlers::CannotStream {
            others: any_streams,
        }
    }
}

/// Turns a pending Java exception into a Rust error (and clears it, so the
/// JNI thread stays usable).
fn take_exception(env: &mut JNIEnv<'_>, what: &str) -> Result<(), String> {
    match env.exception_check() {
        Ok(false) => Ok(()),
        Ok(true) => {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            Err(format!("{what}: Android threw an exception"))
        }
        Err(e) => Err(java_error(e)),
    }
}

/// Hands `url` to the system video player (or the matching app).
///
/// `mime` is `Some("video/*")` for direct media links so Android prefers a
/// video player; YouTube/web links pass `None` and open in the browser or the
/// YouTube app.
pub fn open_video(url: &str, mime: Option<&str>) -> Result<(), String> {
    let vm = vm()?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|e| format!("could not attach to the JVM: {e}"))?;

    let result = (|| -> Result<(), String> {
        let action = env
            .new_string("android.intent.action.VIEW")
            .map_err(java_error)?;
        let url_j = env.new_string(url).map_err(java_error)?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[(&url_j).into()],
            )
            .map_err(java_error)?
            .l()
            .map_err(java_error)?;

        let intent = env
            .new_object(
                "android/content/Intent",
                "(Ljava/lang/String;Landroid/net/Uri;)V",
                &[(&action).into(), (&uri).into()],
            )
            .map_err(java_error)?;

        // A `video/*` intent only helps if the player can fetch the URL
        // itself; galleries that only read local media would just close again.
        let mut mime = if url.starts_with("http://") || url.starts_with("https://") {
            match video_handlers(&mut env, &intent) {
                VideoHandlers::Streams => mime,
                VideoHandlers::None => {
                    eprintln!("[scwoll] no video/* app installed — opening in the browser");
                    None
                }
                VideoHandlers::CannotStream { others } => {
                    if others {
                        // Several players, none of them the default: let the
                        // user pick one that can stream.
                        eprintln!("[scwoll] default video app can't stream — showing a chooser");
                        let title = env.new_string("Play with").map_err(java_error)?;
                        let chooser = env
                            .call_static_method(
                                "android/content/Intent",
                                "createChooser",
                                "(Landroid/content/Intent;Ljava/lang/CharSequence;)Landroid/content/Intent;",
                                &[(&intent).into(), (&title).into()],
                            )
                            .map_err(java_error)?
                            .l()
                            .map_err(java_error)?;
                        return start(&mut env, &chooser);
                    }
                    eprintln!(
                        "[scwoll] the installed video app can't stream URLs — opening in the browser"
                    );
                    None
                }
            }
        } else {
            mime
        };

        if let Some(m) = mime.take() {
            let mime_j = env.new_string(m).map_err(java_error)?;
            env.call_method(
                &intent,
                "setDataAndType",
                "(Landroid/net/Uri;Ljava/lang/String;)Landroid/content/Intent;",
                &[(&uri).into(), (&mime_j).into()],
            )
            .map_err(java_error)?;
        }

        start(&mut env, &intent)
    })();

    // Never leave an exception pending on this thread.
    let _ = env.exception_clear();
    result
}

/// Adds the launch flags and starts the activity.
fn start(env: &mut JNIEnv<'_>, intent: &JObject<'_>) -> Result<(), String> {
    env.call_method(
        intent,
        "addFlags",
        "(I)Landroid/content/Intent;",
        &[JValue::Int(FLAG_ACTIVITY_NEW_TASK | FLAG_GRANT_READ_URI_PERMISSION)],
    )
    .map_err(java_error)?;
    let activity = activity();
    env.call_method(
        &activity,
        "startActivity",
        "(Landroid/content/Intent;)V",
        &[intent.into()],
    )
    .map_err(java_error)?;
    take_exception(env, "startActivity")
}

/// Best-effort JPEG thumbnail for a video URL/local path, via
/// `MediaMetadataRetriever`: first the embedded artwork, then a frame at ~10%
/// of the duration. Returns `None` when Android can't decode the source.
pub fn video_thumb_jpeg(input: &str, target_w: u32) -> Option<Vec<u8>> {
    let vm = vm().ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let out = video_thumb_inner(&mut env, input, target_w);
    let _ = env.exception_clear();
    out
}

fn video_thumb_inner(env: &mut JNIEnv<'_>, input: &str, target_w: u32) -> Option<Vec<u8>> {
    let retriever = env
        .new_object("android/media/MediaMetadataRetriever", "()V", &[])
        .ok()?;
    let src = env.new_string(input).ok()?;
    env.call_method(
        &retriever,
        "setDataSource",
        "(Ljava/lang/String;)V",
        &[(&src).into()],
    )
    .ok()?;
    if env.exception_check().unwrap_or(true) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return None;
    }

    // Embedded artwork if the file carries any, else a frame 10% in.
    let embedded = env
        .call_method(&retriever, "getEmbeddedPicture", "()[B", &[])
        .ok()
        .and_then(|v| v.l().ok())
        .filter(|o| !o.is_null());

    let mut bitmap = embedded.and_then(|bytes| {
        let len = byte_array_len(env, &bytes);
        if len <= 0 {
            return None;
        }
        env.call_static_method(
            "android/graphics/BitmapFactory",
            "decodeByteArray",
            "([BII)Landroid/graphics/Bitmap;",
            &[(&bytes).into(), JValue::Int(0), JValue::Int(len)],
        )
        .ok()
        .and_then(|v| v.l().ok())
        .filter(|o| !o.is_null())
    });

    if bitmap.is_none() {
        let duration_ms = env
            .call_method(
                &retriever,
                "extractMetadata",
                "(I)Ljava/lang/String;",
                &[JValue::Int(METADATA_KEY_DURATION)],
            )
            .ok()
            .and_then(|v| v.l().ok())
            .filter(|o| !o.is_null())
            .and_then(|s| {
                let s = jni::objects::JString::from(s);
                let text: String = env.get_string(&s).ok()?.into();
                text.trim().parse::<i64>().ok()
            })
            .unwrap_or(0);
        let at_us = (duration_ms.max(1) * 1000 / 10) as i64;
        bitmap = env
            .call_method(
                &retriever,
                "getFrameAtTime",
                "(JI)Landroid/graphics/Bitmap;",
                &[JValue::Long(at_us), JValue::Int(OPTION_CLOSEST_SYNC)],
            )
            .ok()
            .and_then(|v| v.l().ok())
            .filter(|o| !o.is_null());
    }

    let bitmap = bitmap?;
    let _ = env.call_method(&retriever, "release", "()V", &[]);

    // Scale down (keeping the aspect ratio) so the JPEG stays small.
    let width = env
        .call_method(&bitmap, "getWidth", "()I", &[])
        .ok()?
        .i()
        .ok()?;
    let height = env
        .call_method(&bitmap, "getHeight", "()I", &[])
        .ok()?
        .i()
        .ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    let w = target_w.min(width as u32).max(64) as i32;
    let h = ((w as i64 * height as i64) / width as i64).max(1) as i32;
    let scaled = env
        .call_static_method(
            "android/graphics/Bitmap",
            "createScaledBitmap",
            "(Landroid/graphics/Bitmap;IIZ)Landroid/graphics/Bitmap;",
            &[(&bitmap).into(), JValue::Int(w), JValue::Int(h), JValue::Bool(1)],
        )
        .ok()?
        .l()
        .ok()?;

    let stream = env
        .new_object("java/io/ByteArrayOutputStream", "()V", &[])
        .ok()?;
    let format_class = env.find_class("android/graphics/Bitmap$CompressFormat").ok()?;
    let jpeg = env
        .get_static_field(
            &format_class,
            "JPEG",
            "Landroid/graphics/Bitmap$CompressFormat;",
        )
        .ok()?
        .l()
        .ok()?;
    let ok = env
        .call_method(
            &scaled,
            "compress",
            "(Landroid/graphics/Bitmap$CompressFormat;ILjava/io/OutputStream;)Z",
            &[(&jpeg).into(), JValue::Int(82), (&stream).into()],
        )
        .ok()?
        .z()
        .ok()?;
    if !ok {
        return None;
    }
    let bytes = env
        .call_method(&stream, "toByteArray", "()[B", &[])
        .ok()?
        .l()
        .ok()?;
    let arr = jni::objects::JByteArray::from(bytes);
    env.convert_byte_array(&arr).ok().filter(|b| !b.is_empty())
}

fn byte_array_len(env: &mut JNIEnv<'_>, array: &JObject<'_>) -> i32 {
    // SAFETY: `array` really is a `byte[]` (from `getEmbeddedPicture`).
    let arr = unsafe { jni::objects::JByteArray::from_raw(array.as_raw()) };
    env.get_array_length(&arr).unwrap_or(0)
}

/// Opens a plain web page (used by the Sources "open in browser" actions).
pub fn open_url(url: &str) -> Result<(), String> {
    open_video(url, None)
}

/// Height of the system bars in physical pixels, as `(top, bottom)`.
///
/// With the status/navigation bars visible the app window still spans the whole
/// screen, so the phone UI pads its app bar and bottom navigation by these.
/// Returns `(0, 0)` when the insets can't be read.
pub fn system_bar_insets_px() -> (f32, f32) {
    let Ok(vm) = vm() else {
        return (0.0, 0.0);
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return (0.0, 0.0);
    };
    let insets = read_insets(&mut env);
    let _ = env.exception_clear();
    insets.unwrap_or((0.0, 0.0))
}

fn read_insets(env: &mut JNIEnv<'_>) -> Option<(f32, f32)> {
    let activity = activity();
    let window = env
        .call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])
        .ok()?
        .l()
        .ok()?;
    let decor = env
        .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])
        .ok()?
        .l()
        .ok()?;
    let insets = env
        .call_method(
            &decor,
            "getRootWindowInsets",
            "()Landroid/view/WindowInsets;",
            &[],
        )
        .ok()?
        .l()
        .ok()?;
    if insets.is_null() {
        return None;
    }

    // API 30+: WindowInsets.getInsets(WindowInsets.Type.statusBars())
    let typed = (|| -> Option<(f32, f32)> {
        let status = env
            .call_static_method(
                "android/view/WindowInsets$Type",
                "statusBars",
                "()I",
                &[],
            )
            .ok()?
            .i()
            .ok()?;
        let nav = env
            .call_static_method(
                "android/view/WindowInsets$Type",
                "navigationBars",
                "()I",
                &[],
            )
            .ok()?
            .i()
            .ok()?;
        let top = insets_side(env, &insets, status, "top")?;
        let bottom = insets_side(env, &insets, nav, "bottom")?;
        Some((top, bottom))
    })();
    if typed.is_some() {
        return typed;
    }
    let _ = env.exception_clear();

    // API < 30 fallback (deprecated but still present).
    let top = env
        .call_method(&insets, "getSystemWindowInsetTop", "()I", &[])
        .ok()?
        .i()
        .ok()?;
    let bottom = env
        .call_method(&insets, "getSystemWindowInsetBottom", "()I", &[])
        .ok()?
        .i()
        .ok()?;
    Some((top as f32, bottom as f32))
}

fn insets_side(
    env: &mut JNIEnv<'_>,
    window_insets: &JObject<'_>,
    kind: i32,
    side: &str,
) -> Option<f32> {
    let insets = env
        .call_method(
            window_insets,
            "getInsets",
            "(I)Landroid/graphics/Insets;",
            &[JValue::Int(kind)],
        )
        .ok()?
        .l()
        .ok()?;
    if insets.is_null() {
        return None;
    }
    let value = env
        .get_field(&insets, side, "I")
        .ok()?
        .i()
        .ok()?;
    Some(value as f32)
}
