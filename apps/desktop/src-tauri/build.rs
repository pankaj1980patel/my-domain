use std::env;
use std::path::PathBuf;

fn main() {
    // Build scripts run on the host: the target comes from Cargo's env vars.
    let windows_msvc = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    let mut attributes = tauri_build::Attributes::new();
    if windows_msvc {
        // The linker embeds the app manifest instead (see below), so
        // tauri-build must not add a second one to its resource file.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        embed_windows_manifest();
    }
    if let Err(error) = tauri_build::try_build(attributes) {
        println!("{error:#}");
        std::process::exit(1);
    }
}

/// tauri-build puts the app manifest (the Common Controls v6 dependency that
/// comctl32 entry points like `TaskDialogIndirect` need) into a resource that
/// is linked into the app binary only (`rustc-link-arg-bins`). The test
/// executables then fail to start with STATUS_ENTRYPOINT_NOT_FOUND. Having
/// the linker embed the same manifest (tauri-build 2.6.2's default, copied
/// verbatim) into every binary this package links fixes that; it is what Tauri
/// does for its own tests:
/// https://github.com/tauri-apps/tauri/pull/4383#issuecomment-1212221864
fn embed_windows_manifest() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    // The plain (all-targets) form: `rustc-link-arg-tests` covers only
    // integration tests, not the lib's unit-test executable.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
