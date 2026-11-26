# Android arm64 build shell for Scwoll.
#
# Provides:
#   - rustup (cargo/rustc + `rustup target add aarch64-linux-android`)
#   - Android SDK (platform-tools, build-tools, platform android-34)
#   - Android NDK (arm64 toolchain)
#   - adb
#
# The Android SDK/NDK packages are `unfree` and require accepting Google's
# Android SDK license, so set both env vars before entering the shell:
#
#   export NIXPKGS_ALLOW_UNFREE=1
#   export NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE=1
#   nix-shell shell-android.nix
#
# Then, inside the shell (one-time toolchain setup):
#
#   rustup default stable
#   ./build-apk.sh
#
# First entry downloads the SDK + NDK (several GB) from Google, and the first
# `rustup default stable` downloads the Rust toolchain.

{ pkgs ? import <nixpkgs> { } }:

let
  # One standard SDK/NDK composition: android-34 + build-tools 34 + NDK r30.
  android = pkgs.androidenv.composeAndroidPackages {
    includeNDK = true;
    includeEmulator = false;
    includeSystemImages = false;
    includeCmake = false;
    buildToolsVersions = [ "34.0.0" ];
    platformVersions = [ "34" ];
  };

  # The nix androidenv `androidsdk` derivation puts the *real* SDK root (with
  # build-tools/, platforms/, platform-tools/, ndk-bundle) under
  # `libexec/android-sdk`, not at the derivation root.
  androidSdk = "${android.androidsdk}/libexec/android-sdk";

  # `cargo apk` generates the debug keystore with `keytool`, and apksigner is
  # a Java program, so a JDK must be on PATH (and JAVA_HOME set).
  jdk = pkgs.jdk17;
in
pkgs.mkShell {
  buildInputs = [
    pkgs.rustup
    pkgs.android-tools # adb
    jdk # keytool + apksigner backend
    pkgs.stdenv.cc # host `cc` for build scripts / proc-macros
    pkgs.zlib # link-time deps of some crates
    pkgs.pkg-config
  ];

  # cargo-apk / ndk-build read these.
  ANDROID_HOME = androidSdk;
  ANDROID_NDK_HOME = "${androidSdk}/ndk-bundle";
  ANDROID_NDK_ROOT = "${androidSdk}/ndk-bundle";
  ANDROID_SDK_ROOT = androidSdk;
  JAVA_HOME = "${jdk}";

  shellHook = ''
    echo "📱 Scwoll Android shell"
    echo "   one-time: rustup default stable"
    echo "   build:    ./build-apk.sh              (release, signed with ~/.android/scwoll-release.keystore)"
    echo "   debug:    PROFILE=dev ./build-apk.sh  (symbols for adb logcat backtraces)"
    echo "   install:  adb install -r target/release/apk/scwoll.apk"
  '';
}
