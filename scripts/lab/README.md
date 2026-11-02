# Device lab

Real-device testing for my-domain. The topology:

```
 Mac (dev + macOS node) ──ssh/rsync──▶ Arch laptop 192.168.0.176 (Linux node, Android build host)
                                              │ USB
                                              ▼
                                       Android phone (I2405)
```

The inventory is in [lab.toml](lab.toml). Hosts can be overridden through the
environment variables in [common.sh](common.sh).

## First time
1. Install your SSH key so the scripts don't keep asking for a password:
   `ssh-copy-id pankaj@192.168.0.176`
2. Install the toolchain on the laptop (JDK 17, Android SDK/NDK, Rust Android
   targets, Tauri deps): `scripts/lab/setup-arch.sh`

## Everyday use
| Command | What it does |
|---|---|
| `scripts/lab/sync.sh` | Copy the working tree to the laptop (uncommitted changes included) |
| `scripts/lab/linux.sh test` | Run `cargo test --workspace` on Linux |
| `scripts/lab/linux.sh desktop` | Run the desktop app on the laptop |
| `scripts/lab/android.sh install` | Build the APK on the laptop, install it on the phone and launch it |
| `scripts/lab/android.sh test` | Run instrumentation tests on the phone |
| `scripts/lab/android.sh logcat` | Stream the app's logs |

Cross-device scenario tests (`scenarios/*.mdctl`, driven by `mdctl`) are
described below.

## mdctl: a scriptable device

`apps/mdctl` runs a my-domain device with no UI. It prints every event and
command result as one JSON line, and takes commands on stdin:

```sh
set -a; . scripts/lab/test-creds.env; set +a
printf 'peers\nsend <node_id> hello\nquit\n' | cargo run -q -p mdctl -- --name lab-box --data /tmp/lab-box
```

Commands: `identity`, `peers`, `links`, `refresh`, `connect <node>`,
`send <node> <text>`, `status <node>`, `ring <node>`, `open <node> <url>`,
`sendfile <node> <path>`, `downloads <dir>` (where received files go),
`action <node> <name> [json]`, `add <name> <ip> <quic_port>`, `forget` (remove
this device from the server), `quit`. `--offline USER --key K` skips the server.

### Scenarios

`--script FILE` runs a scenario; the first failed command or `wait` exits 1.
They live in [scenarios/](scenarios) and take the peer's name from
`MD_PEER` (the phone is `I2405`; a Mac or another mdctl works too):

| Scenario | Checks |
|---|---|
| `chat-roundtrip.mdctl` | message to an `echo on` mdctl comes back |
| `wake-by-push.mdctl` | a sleeping device is woken by push and the held message goes out |
| `receipts.mdctl` | a message gets a `message-delivered` receipt |
| `file-transfer.mdctl` | `MD_FILE` is sent and confirmed (`file-sent`) |
| `controls.mdctl` | `status`, volume +1 then -1, `device-status-event` (no ring, no flashlight) |

```sh
set -a; . scripts/lab/test-creds.env; set +a
MD_PEER=I2405 MD_FILE=/tmp/lab-file.bin cargo run -q -p mdctl -- \
  --name lab-box --data /tmp/lab-box --script scripts/lab/scenarios/file-transfer.mdctl
```

`cargo test -p mdctl` runs the same kinds of scenario offline between two
local mdctl processes (`apps/mdctl/tests/scenario.rs`).

## Android builds on the Mac

The Mac can build and install the app too (SDK in `~/Library/Android/sdk`,
`apps/android/local.properties` points at it; JDK 17):

```sh
cd apps/android
JAVA_HOME=$(/usr/libexec/java_home -v 17) ./gradlew assembleDebug -Prust.targets=arm64
~/Library/Android/sdk/platform-tools/adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`-Prust.targets=arm64` builds the core for the phone only (the default
builds all four ABIs). The Mac and the laptop sign debug builds with
different keys, so switching hosts needs `adb uninstall com.mydomain.android`
first, and the phone gets a new node id (Android scopes `ANDROID_ID` to the
signing key).


## Reading the phone's core events

Some phones (vivo) rate-limit logcat and mute a chatty app. Debug builds
also append every core event, push and visibility change to a file:

```sh
adb shell run-as com.mydomain.android cat files/events.log
```
