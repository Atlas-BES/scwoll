#!/usr/bin/env bash
# Build Scwoll as an arm64 (aarch64) Android APK.
#
# Prerequisites (shell-android.nix provides all of them):
#   1. rustup          -> cargo/rustc + the aarch64-linux-android std target
#   2. Android NDK     -> export ANDROID_NDK_HOME (or ANDROID_NDK_ROOT)
#   3. Android SDK     -> export ANDROID_HOME     (or ANDROID_SDK_ROOT)
#      (cargo-apk needs aapt/aapt2 + apksigner from build-tools, and
#       android.jar from platforms/android-<target_sdk_version>)
#   4. JDK             -> `keytool` (keystore) + `java` (apksigner)
#   5. cargo-apk       -> installed automatically below if missing
#
# Default: RELEASE APK (smaller, faster, and it avoids the rustc debug-only UB
# precondition check that makes `android-activity` abort on startup).
# A local signing key is generated on first use at
# ~/.android/scwoll-release.keystore (password: android).
#
#     ./build-apk.sh
#     adb install -r target/release/apk/scwoll.apk
#
# For a debug build with symbols (backtraces in `adb logcat`):
#
#     PROFILE=dev ./build-apk.sh
#     adb install -r target/android-dev/apk/scwoll.apk
#
# Environment:
#   PROFILE=dev|release            (default: release)
#   CARGO_APK_RELEASE_KEYSTORE     use an existing signing keystore
#   CARGO_APK_RELEASE_KEYSTORE_PASSWORD
#   KEYSTORE_PASSWORD              password for the auto-generated keystore

set -euo pipefail
cd "$(dirname "$0")"

info() { printf '\033[1;34m[apk]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[apk] error:\033[0m %s\n' "$*" >&2; exit 1; }

# --- rustup ----------------------------------------------------------------
command -v cargo >/dev/null 2>&1 || die "cargo not found (enter the Android shell: nix-shell shell-android.nix)"

if command -v rustup >/dev/null 2>&1; then
    if ! rustup show active-toolchain >/dev/null 2>&1; then
        info "no default Rust toolchain — installing stable"
        rustup default stable
    fi
    info "adding aarch64-linux-android std target"
    rustup target add aarch64-linux-android
else
    info "rustup not found — assuming the aarch64-linux-android std target is already installed"
fi

# --- cargo-apk --------------------------------------------------------------
if ! command -v cargo-apk >/dev/null 2>&1; then
    info "installing cargo-apk"
    cargo install cargo-apk
fi

# --- Android NDK -----------------------------------------------------------
if [ -z "${ANDROID_NDK_HOME:-}" ] && [ -n "${ANDROID_NDK_ROOT:-}" ]; then
    export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"
fi
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    die "set ANDROID_NDK_HOME to your Android NDK (e.g. ~/Android/Sdk/ndk/<version>)"
fi
info "using NDK: ${ANDROID_NDK_HOME}"

# --- Android SDK -----------------------------------------------------------
if [ -z "${ANDROID_HOME:-}" ] && [ -n "${ANDROID_SDK_ROOT:-}" ]; then
    export ANDROID_HOME="$ANDROID_SDK_ROOT"
fi
if [ -z "${ANDROID_HOME:-}" ]; then
    die "set ANDROID_HOME to your Android SDK (e.g. ~/Android/Sdk)"
fi
info "using SDK: ${ANDROID_HOME}"

# --- JDK -------------------------------------------------------------------
# cargo-apk shells out to `keytool` (keystores) and to `java` (apksigner).
# Find one even if the shell didn't put it on PATH.
if ! command -v keytool >/dev/null 2>&1; then
    jdk_candidates=()
    if [ -n "${JAVA_HOME:-}" ]; then
        jdk_candidates+=("$JAVA_HOME/bin/keytool")
    fi
    if command -v java >/dev/null 2>&1; then
        jdk_candidates+=("$(dirname "$(readlink -f "$(command -v java)")")/keytool")
    fi
    # Android Studio's bundled JetBrains Runtime
    jdk_candidates+=(
        "${ANDROID_HOME}/../jbr/bin/keytool"
        "$HOME/android-studio/jbr/bin/keytool"
        "/opt/android-studio/jbr/bin/keytool"
        "/usr/lib/jvm/default/bin/keytool"
    )
    # Nix store OpenJDK (e.g. when JAVA_HOME was not exported)
    nix_jdk="$(ls -d /nix/store/*-openjdk-*-bin/keytool /nix/store/*-openjdk-*/bin/keytool 2>/dev/null | head -n1 || true)"
    if [ -n "$nix_jdk" ]; then
        jdk_candidates+=("$nix_jdk")
    fi

    for candidate in "${jdk_candidates[@]}"; do
        if [ -n "$candidate" ] && [ -x "$candidate" ]; then
            export JAVA_HOME="$(dirname "$(dirname "$candidate")")"
            export PATH="$JAVA_HOME/bin:$PATH"
            info "using JDK: $JAVA_HOME"
            break
        fi
    done
fi
if ! command -v keytool >/dev/null 2>&1; then
    die "keytool not found — cargo-apk needs a JDK. Enter the Nix shell (it provides jdk17): nix-shell shell-android.nix"
fi

# --- profile ---------------------------------------------------------------
case "${PROFILE:-release}" in
    dev | debug) CARGO_PROFILE=android-dev ;;
    release) CARGO_PROFILE=release ;;
    *) die "PROFILE must be 'dev' or 'release' (got '$PROFILE')" ;;
esac
info "profile: $CARGO_PROFILE"

# --- signing ---------------------------------------------------------------
# cargo-apk picks the keystore from an env var named after the profile:
#   CARGO_APK_RELEASE_KEYSTORE       (+ _PASSWORD)  for `--release`
#   CARGO_APK_ANDROID_DEV_KEYSTORE   (+ _PASSWORD)  for `--profile android-dev`
if [ "$CARGO_PROFILE" = "release" ]; then
    KEYSTORE_VAR=CARGO_APK_RELEASE_KEYSTORE
    DEFAULT_KEYSTORE="$HOME/.android/scwoll-release.keystore"
else
    KEYSTORE_VAR=CARGO_APK_ANDROID_DEV_KEYSTORE
    DEFAULT_KEYSTORE="$HOME/.android/scwoll-debug.keystore"
fi
PASSWORD_VAR="${KEYSTORE_VAR}_PASSWORD"

KEYSTORE="${!KEYSTORE_VAR:-$DEFAULT_KEYSTORE}"
PASSWORD="${!PASSWORD_VAR:-${KEYSTORE_PASSWORD:-android}}"

if [ ! -f "$KEYSTORE" ] && [ -z "${!KEYSTORE_VAR:-}" ]; then
    info "generating signing keystore: $KEYSTORE"
    mkdir -p "$(dirname "$KEYSTORE")"
    keytool -genkeypair -v \
        -keystore "$KEYSTORE" \
        -alias scwoll \
        -keyalg RSA -keysize 2048 -validity 10000 \
        -storepass "$PASSWORD" -keypass "$PASSWORD" \
        -dname "CN=Scwoll, O=Scwoll, C=US" >/dev/null
fi
[ -f "$KEYSTORE" ] || die "signing keystore not found: $KEYSTORE"
export "$KEYSTORE_VAR=$KEYSTORE"
export "$PASSWORD_VAR=$PASSWORD"
info "signing with: $KEYSTORE"

# --- build -----------------------------------------------------------------
info "building $CARGO_PROFILE APK for aarch64-linux-android"
# `--lib` is required: cargo-apk 0.10 panics ("Bin is not compatible with
# Cdylib") when the package also has a `[[bin]]` target, and the APK only
# needs the cdylib whose `android_main` is the entry point.
cargo apk build --profile "$CARGO_PROFILE" --lib

APK="target/$CARGO_PROFILE/apk/scwoll.apk"
if [ ! -f "$APK" ]; then
    die "APK not found at $APK after build"
fi

info "APK: $APK ($(du -h "$APK" | cut -f1))"
info "install: adb install -r \"$APK\""
info "launch:  adb shell am start -n dev.scwoll.app/android.app.NativeActivity"
info "logs:    adb logcat -d -s RustStdoutStderr:V AndroidRuntime:E DEBUG:E libc:F"
