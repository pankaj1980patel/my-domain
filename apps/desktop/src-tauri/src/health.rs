//! First-run setup and the health check on the computer (the core's
//! `features::onboarding` and `features::health`): what only this app can
//! read (macOS permissions, a screenshot tool on Linux, free space where
//! received files go) goes to the core, which adds the server, pushes,
//! links, keys and versions and rates each line. Each permission's fix
//! opens the OS's own page for it: nothing is ever granted from here.

use mdcore::features::health::{HealthReport, PermissionFact, PlatformFacts};
use mdcore::features::onboarding::Onboarding;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

pub fn this_os() -> Os {
    if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(target_os = "windows") {
        Os::Windows
    } else {
        Os::Linux
    }
}

/// What was read here (none: can't be read on this OS).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readings {
    /// macOS Privacy & Security › Accessibility (input from your phone,
    /// shared mouse and keyboard, paste for you).
    pub accessibility: Option<bool>,
    /// Snapshots of the screen can be taken: Ok, or why not (macOS
    /// "screen_recording"; Linux "grim" / "imagemagick", a tool to
    /// install).
    pub screen: Option<Result<(), String>>,
}

fn fact(id: &str, features: &[&str], state: &str, critical: bool) -> PermissionFact {
    PermissionFact {
        id: id.into(),
        features: features.iter().map(|s| s.to_string()).collect(),
        state: state.into(),
        critical,
    }
}

fn on_off(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "granted",
        Some(false) => "denied",
        None => "unknown",
    }
}

/// Each permission or setting the features need on `os`, and how it
/// stands. Those macOS doesn't let an app read (Local Network, Bluetooth,
/// Notifications) are "unknown": shown grey, with their page.
pub fn permission_facts(os: Os, r: &Readings) -> Vec<PermissionFact> {
    let mut out = Vec::new();
    match os {
        Os::Mac => {
            out.push(fact("local_network", &[], "unknown", false));
            out.push(fact("notifications", &["notifications"], "unknown", false));
            out.push(fact(
                "accessibility",
                &["media", "clipboard"],
                on_off(r.accessibility),
                false,
            ));
            let screen = match &r.screen {
                Some(Ok(())) => "granted",
                Some(Err(_)) => "denied",
                None => "unknown",
            };
            out.push(fact("screen_recording", &["media"], screen, true));
            out.push(fact("bluetooth", &["media"], "unknown", false));
        }
        Os::Windows => {
            out.push(fact("notifications", &["notifications"], "unknown", false));
            out.push(fact("location", &["find"], "unknown", false));
        }
        Os::Linux => {
            // A screenshot tool to install, when one is missing.
            if let Some(Err(tool)) = &r.screen {
                if tool == "grim" || tool == "imagemagick" {
                    out.push(fact(&format!("tool_{tool}"), &["media"], "denied", false));
                }
            }
        }
    }
    out
}

/// The OS page where the user turns `id` on (none: no page to open).
pub fn settings_url(os: Os, id: &str) -> Option<&'static str> {
    Some(match (os, id) {
        (Os::Mac, "accessibility") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        (Os::Mac, "screen_recording") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        (Os::Mac, "local_network") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork"
        }
        (Os::Mac, "bluetooth") => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"
        }
        (Os::Mac, "notifications") => {
            "x-apple.systempreferences:com.apple.preference.notifications"
        }
        (Os::Windows, "notifications") => "ms-settings:notifications",
        (Os::Windows, "location") => "ms-settings:privacy-location",
        _ => return None,
    })
}

/// Bytes to whole MB, for the report.
pub fn to_mb(bytes: u64) -> u32 {
    u32::try_from(bytes / (1024 * 1024)).unwrap_or(u32::MAX)
}

fn readings() -> Readings {
    Readings {
        #[cfg(target_os = "macos")]
        accessibility: Some(crate::device::accessibility_trusted()),
        #[cfg(not(target_os = "macos"))]
        accessibility: None,
        screen: Some(crate::screen_share::ready().map_err(String::from)),
    }
}

fn facts(app: &AppHandle) -> PlatformFacts {
    let disk = app
        .try_state::<crate::DownloadDir>()
        .and_then(|d| crate::device::storage_of(&d.0));
    PlatformFacts {
        permissions: permission_facts(this_os(), &readings()),
        disk_free_mb: disk.map(|(free, _)| to_mb(free)),
        disk_total_mb: disk.map(|(_, total)| to_mb(total)),
    }
}

// --- commands ---

/// The health check. `check_server` asks the server too (a few seconds),
/// so it runs off the UI's thread.
#[tauri::command]
#[specta::specta]
pub async fn health_check(check_server: bool, app: AppHandle) -> Result<HealthReport, CoreError> {
    tauri::async_runtime::spawn_blocking(move || {
        let facts = facts(&app);
        app.state::<Eng>().health(&facts, check_server)
    })
    .await
    .map_err(|e| CoreError::Other(e.to_string()))
}

/// Open the OS page for permission `id` (the health check's and setup's
/// "Open settings"). False: this OS has no such page.
#[tauri::command]
#[specta::specta]
pub fn open_permission_page(id: String, app: AppHandle) -> Result<bool, CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let Some(url) = settings_url(this_os(), &id) else {
        return Ok(false);
    };
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))?;
    Ok(true)
}

/// Open the folder received files go to (the disk check's fix).
#[tauri::command]
#[specta::specta]
pub fn open_downloads(app: AppHandle) -> Result<(), CoreError> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app
        .try_state::<crate::DownloadDir>()
        .map(|d| d.0.clone())
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| CoreError::Other("no downloads folder".into()))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| CoreError::Other(e.to_string()))
}

/// Where first-run setup stands on this computer.
#[tauri::command]
#[specta::specta]
pub fn onboarding_state(engine: State<Eng>) -> Onboarding {
    engine.onboarding()
}

/// A tap in first-run setup ("next", "back", "skip", "goto" with `step`,
/// "skip_all", "finish", "restart").
#[tauri::command]
#[specta::specta]
pub fn onboarding_act(
    action: String,
    step: Option<String>,
    engine: State<Eng>,
) -> Result<Onboarding, CoreError> {
    engine.onboarding_act(&action, step.as_deref())
}

/// The feature groups picked in setup.
#[tauri::command]
#[specta::specta]
pub fn set_onboarding_features(features: Vec<String>, engine: State<Eng>) -> Onboarding {
    engine.set_onboarding_features(&features)
}

/// Name this computer (setup's first step). Returns the name as kept.
#[tauri::command]
#[specta::specta]
pub fn set_device_name(name: String, engine: State<Eng>) -> Result<String, CoreError> {
    engine.set_device_name(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(f: &[PermissionFact]) -> Vec<(&str, &str)> {
        f.iter()
            .map(|p| (p.id.as_str(), p.state.as_str()))
            .collect()
    }

    #[test]
    fn mac_reads_what_it_can_and_points_at_the_rest() {
        let r = Readings {
            accessibility: Some(false),
            screen: Some(Err("screen_recording".into())),
        };
        let f = permission_facts(Os::Mac, &r);
        assert_eq!(
            states(&f),
            vec![
                ("local_network", "unknown"),
                ("notifications", "unknown"),
                ("accessibility", "denied"),
                ("screen_recording", "denied"),
                ("bluetooth", "unknown"),
            ]
        );
        assert!(
            f.iter()
                .find(|p| p.id == "screen_recording")
                .unwrap()
                .critical
        );
        assert_eq!(
            f.iter().find(|p| p.id == "accessibility").unwrap().features,
            vec!["media", "clipboard"]
        );
        let ok = Readings {
            accessibility: Some(true),
            screen: Some(Ok(())),
        };
        let f = permission_facts(Os::Mac, &ok);
        assert!(f
            .iter()
            .filter(|p| p.state != "unknown")
            .all(|p| p.state == "granted"));
        // Every one has a page to open.
        assert!(f.iter().all(|p| settings_url(Os::Mac, &p.id).is_some()));
    }

    #[test]
    fn linux_only_asks_for_a_missing_screenshot_tool() {
        let none = Readings {
            accessibility: None,
            screen: Some(Ok(())),
        };
        assert!(permission_facts(Os::Linux, &none).is_empty());
        let grim = Readings {
            accessibility: None,
            screen: Some(Err("grim".into())),
        };
        assert_eq!(
            states(&permission_facts(Os::Linux, &grim)),
            vec![("tool_grim", "denied")]
        );
        assert_eq!(settings_url(Os::Linux, "tool_grim"), None);
    }

    #[test]
    fn windows_pages() {
        let f = permission_facts(Os::Windows, &Readings::default());
        assert_eq!(
            states(&f),
            vec![("notifications", "unknown"), ("location", "unknown")]
        );
        assert_eq!(
            settings_url(Os::Windows, "location"),
            Some("ms-settings:privacy-location")
        );
        assert_eq!(settings_url(Os::Windows, "accessibility"), None);
    }

    #[test]
    fn megabytes() {
        assert_eq!(to_mb(5 * 1024 * 1024 + 3), 5);
        assert_eq!(to_mb(u64::MAX), u32::MAX);
    }
}
