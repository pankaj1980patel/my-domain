#!/usr/bin/env bash
# Runs ON THE ARCH HOST (called by scripts/lab/android.sh coretest).
# Cross-compiles mdcore's test binaries for the phone and runs them there.
#   usage: android-coretest.sh <adb-serial> [test-name-filter]
set -euo pipefail
SERIAL="$1"
FILTER="${2:-}"
ADB="adb -s $SERIAL"
TARGET=aarch64-linux-android
API=24   # = minSdk in apps/android/app/build.gradle.kts

NDK=$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)
BIN="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
# Linker for Rust code, and C compiler/archiver for crates that build C code
# through the `cc` crate (env var names use the target with '-' -> '_').
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$BIN/${TARGET}${API}-clang"
export CC_aarch64_linux_android="$BIN/${TARGET}${API}-clang"
export AR_aarch64_linux_android="$BIN/llvm-ar"

echo "--- building test binaries for $TARGET"
mapfile -t bins < <(cargo test -p mdcore --target "$TARGET" --no-run --message-format=json \
  | grep -o '"executable":"[^"]*"' | cut -d'"' -f4)

status=0
for b in "${bins[@]}"; do
  name=$(basename "$b")
  echo "--- $name (running on $($ADB shell getprop ro.product.model | tr -d '\r'))"
  # /data/local/tmp is the only place `adb shell` may execute files.
  $ADB push -q "$b" "/data/local/tmp/$name"
  $ADB shell "cd /data/local/tmp && ./$name $FILTER" || status=1
  $ADB shell rm -f "/data/local/tmp/$name"
done
exit $status
