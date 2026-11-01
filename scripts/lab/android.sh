#!/usr/bin/env bash
# Build the Android app on the arch host and drive the phone attached to it.
#   scripts/lab/android.sh build    # assembleDebug
#   scripts/lab/android.sh install  # build + install on the phone + launch
#   scripts/lab/android.sh start    # install + launch logged in with the test account
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
  start)
    # Creds come from the synced test-creds.env on the laptop (not this command line).
    remote ". scripts/lab/test-creds.env && cd apps/android && ANDROID_SERIAL=$MD_ANDROID_SERIAL ./gradlew --no-daemon installDebug &&
      $ADB shell am force-stop $APP_ID &&
      $ADB shell am start -n $APP_ID/.MainActivity -e md_server \$MD_DEV_SERVER -e md_user \$MD_DEV_USER -e md_pass \$MD_DEV_PASS -e md_key \$MD_DEV_KEY" ;;
  test)    remote "cd apps/android && ANDROID_SERIAL=$MD_ANDROID_SERIAL ./gradlew --no-daemon connectedDebugAndroidTest" ;;
  coretest)
    # Run the Rust core's test binaries ON THE PHONE (cross-compiled with the
    # NDK, pushed with adb). Logic lives in remote/android-coretest.sh, which
    # runs on the arch host; keeping it in a file avoids nested-quoting bugs.
    remote "bash scripts/lab/remote/android-coretest.sh $MD_ANDROID_SERIAL ${2:-}" ;;
  *) echo "usage: $0 {build|install|start|test|coretest [filter]|logcat|devices}"; exit 1 ;;
esac
