#!/usr/bin/env bash
# Build the Android app on the arch host and drive the phone attached to it.
#   scripts/lab/android.sh build    # assembleDebug
#   scripts/lab/android.sh install  # build + install on the phone + launch
#   scripts/lab/android.sh test     # instrumentation tests on the phone
#   scripts/lab/android.sh logcat   # stream app logs
#   scripts/lab/android.sh devices  # list attached devices
source "$(dirname "$0")/common.sh"

cmd="${1:-install}"
ADB="adb -s $MD_ANDROID_SERIAL"
APP_ID="com.mydomain.android"

case "$cmd" in
  devices) remote "adb devices -l"; exit 0 ;;
  logcat)  remote "$ADB logcat --pid=\$($ADB shell pidof -s $APP_ID) -v color"; exit 0 ;;
esac

"$(dirname "$0")/sync.sh"

case "$cmd" in
  build)   remote "cd apps/android && ./gradlew --no-daemon assembleDebug" ;;
  install) remote "cd apps/android && ANDROID_SERIAL=$MD_ANDROID_SERIAL ./gradlew --no-daemon installDebug && $ADB shell monkey -p $APP_ID -c android.intent.category.LAUNCHER 1 >/dev/null" ;;
  test)    remote "cd apps/android && ANDROID_SERIAL=$MD_ANDROID_SERIAL ./gradlew --no-daemon connectedDebugAndroidTest" ;;
  *) echo "usage: $0 {build|install|test|logcat|devices}"; exit 1 ;;
esac
