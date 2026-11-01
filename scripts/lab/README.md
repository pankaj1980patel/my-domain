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

Cross-device scenario tests (`scenarios/*.toml`, driven by `mdctl`) are added in
Phase 3, once `mdctl` exists.
