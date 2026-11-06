// What kind of computer this is (mdcore `features::device_kind`), so your
// other devices draw the right icon for it: a Mac laptop, an iMac, a
// Windows desktop, a Linux laptop...
//
// Found once per start on a background thread and kept in
// `<app config dir>/device-kind.json`, so the next start says it at once:
// - macOS: `sysctl -n hw.model` (MacBook* is a laptop, iMac an all-in-one,
//   Macmini / MacPro a desktop). Newer Macs say only "Mac14,3": then the
//   "Model Name" from `system_profiler SPHardwareDataType`, else whether
//   `pmset -g batt` lists an internal battery.
// - Windows: the SMBIOS chassis types, maker and model through one
//   PowerShell call (Win32_SystemEnclosure, Win32_ComputerSystem,
//   Win32_ComputerSystemProduct, Win32_Battery); a battery when the chassis
//   says nothing useful.
// - Linux: /sys/class/dmi/id (chassis_type, sys_vendor, product_name,
//   product_version), else a battery in /sys/class/power_supply.
//
// The chassis and Mac tables are mdcore's (`form_from_chassis`,
// `form_from_mac_model`); what's here reads the OS and picks.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mdcore::features::device_kind as dk;
use mdcore::model::DeviceKind;
use tauri::{AppHandle, Manager, State};

use crate::Eng;

const CACHE_FILE: &str = "device-kind.json";

/// This OS as the core names it.
pub fn this_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        _ => "",
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
/// The form from the chassis types (first useful one), else from whether
/// there's a battery; "" when neither says.
pub fn pick_form(chassis: &[u32], battery: Option<bool>) -> &'static str {
    if let Some(f) = chassis.iter().find_map(|t| dk::form_from_chassis(*t)) {
        return f;
    }
    match battery {
        Some(true) => "laptop",
        Some(false) => "desktop",
        None => "",
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
/// A model name worth showing: not the firmware's placeholders.
pub fn useful_model(s: &str) -> Option<String> {
    let t = dk::tidy_text(s);
    let lower = t.to_ascii_lowercase();
    let placeholder = t.is_empty()
        || lower.contains("o.e.m")
        || lower.contains("default string")
        || lower.contains("system product name")
        || lower.contains("system version")
        || lower == "none"
        || lower == "not applicable"
        || lower == "unknown";
    (!placeholder).then_some(t)
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
/// Lenovo puts the marketing name ("ThinkPad X1 Carbon Gen 9") in the
/// product version and a type code ("20XW004GUS") in the product name.
pub fn pick_model(maker: &str, name: &str, version: &str) -> String {
    let name = useful_model(name);
    let version = useful_model(version);
    if maker.eq_ignore_ascii_case("lenovo") {
        if let Some(v) = version {
            return v;
        }
    }
    name.unwrap_or_default()
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
/// The chassis types in "9,10" or "{9}" or "9" form.
pub fn parse_chassis(s: &str) -> Vec<u32> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter_map(|p| p.parse().ok())
        .collect()
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
/// What the Windows PowerShell call printed, one value per line:
/// chassis types, maker, model, product version, battery count.
pub fn parse_windows(out: &str) -> DeviceKind {
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    let get = |i: usize| lines.get(i).copied().unwrap_or("");
    let chassis = parse_chassis(get(0));
    let maker = dk::tidy_maker(get(1));
    let battery = get(4).parse::<u32>().ok().map(|n| n > 0);
    DeviceKind {
        os: "windows".into(),
        form: pick_form(&chassis, battery).into(),
        model: pick_model(&maker, get(2), get(3)),
        maker,
        icon: String::new(),
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
/// The Mac's kind from its identifier, its "Model Name" (when the
/// identifier doesn't say) and whether it has a battery.
pub fn mac_kind(identifier: &str, model_name: Option<&str>, battery: Option<bool>) -> DeviceKind {
    let name = model_name
        .map(dk::tidy_text)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| dk::mac_name(identifier));
    let form = dk::form_from_mac_model(identifier)
        .or_else(|| dk::form_from_mac_model(&name))
        .unwrap_or(match battery {
            Some(true) => "laptop",
            Some(false) => "desktop",
            None => "",
        });
    DeviceKind {
        os: "macos".into(),
        form: form.into(),
        model: name,
        maker: "Apple".into(),
        icon: String::new(),
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
/// "Model Name: MacBook Air" in `system_profiler SPHardwareDataType`.
pub fn parse_model_name(out: &str) -> Option<String> {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("Model Name:"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// --- reading the OS ---

#[cfg(target_os = "macos")]
fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(target_os = "macos")]
fn detect() -> DeviceKind {
    let identifier = run("sysctl", &["-n", "hw.model"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if dk::form_from_mac_model(&identifier).is_some() {
        return mac_kind(&identifier, None, None);
    }
    let name = run("system_profiler", &["SPHardwareDataType"]).and_then(|o| parse_model_name(&o));
    if name.as_deref().and_then(dk::form_from_mac_model).is_some() {
        return mac_kind(&identifier, name.as_deref(), None);
    }
    let battery = run("pmset", &["-g", "batt"]).map(|o| o.contains("InternalBattery"));
    mac_kind(&identifier, name.as_deref(), battery)
}

#[cfg(target_os = "windows")]
fn detect() -> DeviceKind {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = "$ErrorActionPreference='SilentlyContinue';\
        $e=(Get-CimInstance Win32_SystemEnclosure).ChassisTypes -join ',';\
        $c=Get-CimInstance Win32_ComputerSystem;\
        $p=Get-CimInstance Win32_ComputerSystemProduct;\
        $b=@(Get-CimInstance Win32_Battery).Count;\
        Write-Output $e; Write-Output $c.Manufacturer; Write-Output $c.Model;\
        Write-Output $p.Version; Write-Output $b";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) if o.status.success() => parse_windows(&String::from_utf8_lossy(&o.stdout)),
        _ => DeviceKind {
            os: "windows".into(),
            ..Default::default()
        },
    }
}

#[cfg(target_os = "linux")]
fn detect() -> DeviceKind {
    let dmi =
        |f: &str| std::fs::read_to_string(format!("/sys/class/dmi/id/{f}")).unwrap_or_default();
    let chassis = parse_chassis(&dmi("chassis_type"));
    let battery = std::fs::read_dir("/sys/class/power_supply")
        .ok()
        .map(|dir| {
            dir.flatten().any(|e| {
                std::fs::read_to_string(e.path().join("type")).is_ok_and(|t| t.trim() == "Battery")
                    && !std::fs::read_to_string(e.path().join("scope"))
                        .is_ok_and(|s| s.trim() == "Device")
            })
        });
    let maker = dk::tidy_maker(&dmi("sys_vendor"));
    DeviceKind {
        os: "linux".into(),
        form: pick_form(&chassis, battery).into(),
        model: pick_model(&maker, &dmi("product_name"), &dmi("product_version")),
        maker,
        icon: String::new(),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn detect() -> DeviceKind {
    DeviceKind::default()
}

fn cache_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(CACHE_FILE))
}

/// Say what was found last time at once, then look again in the
/// background (and keep it for next time).
pub fn init(app: &AppHandle, engine: &Eng) {
    let cached = cache_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<DeviceKind>(&s).ok());
    match cached {
        Some(k) => engine.set_device_kind(k),
        // Nothing yet: at least the OS.
        None => engine.set_device_kind(DeviceKind {
            os: this_os().into(),
            ..Default::default()
        }),
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let kind = dk::sanitize(detect());
        if let Some(p) = cache_path(&app) {
            if let Ok(json) = serde_json::to_string(&kind) {
                let _ = std::fs::write(p, json);
            }
        }
        if let Some(engine) = app.try_state::<Eng>() {
            engine.set_device_kind(kind);
        }
    });
}

// --- commands ---

/// Every one of your devices' kind that's known (its OS, form, model,
/// maker and automatic icon), by node id.
#[tauri::command]
#[specta::specta]
pub fn device_kinds(engine: State<Eng>) -> BTreeMap<String, DeviceKind> {
    engine.device_kinds()
}

/// This computer's own kind (with its icon).
#[tauri::command]
#[specta::specta]
pub fn own_device_kind(engine: State<Eng>) -> Option<DeviceKind> {
    engine.own_device_kind()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chassis_then_battery() {
        assert_eq!(pick_form(&[10], None), "laptop");
        assert_eq!(pick_form(&[3], Some(true)), "desktop");
        assert_eq!(pick_form(&[13], None), "all_in_one");
        // "Other" / "Unknown" say nothing: the battery does.
        assert_eq!(pick_form(&[1], Some(true)), "laptop");
        assert_eq!(pick_form(&[2], Some(false)), "desktop");
        assert_eq!(pick_form(&[], None), "");
        assert_eq!(parse_chassis("9,10"), vec![9, 10]);
        assert_eq!(parse_chassis("{3}"), vec![3]);
        assert_eq!(parse_chassis("10\n"), vec![10]);
    }

    #[test]
    fn windows_output() {
        let k = parse_windows("10\nLENOVO\n20XW004GUS\nThinkPad X1 Carbon Gen 9\n1\n");
        assert_eq!(
            (
                k.os.as_str(),
                k.form.as_str(),
                k.maker.as_str(),
                k.model.as_str()
            ),
            ("windows", "laptop", "Lenovo", "ThinkPad X1 Carbon Gen 9")
        );
        assert_eq!(dk::sanitize(k).icon, "win_laptop");
        let d = parse_windows("3\nSystem manufacturer\nSystem Product Name\nSystem Version\n0\n");
        assert_eq!(
            (d.form.as_str(), d.maker.as_str(), d.model.as_str()),
            ("desktop", "", "")
        );
        let b = parse_windows("2\nDell Inc.\nXPS 13 9310\n\n1\n");
        assert_eq!(
            (b.form.as_str(), b.model.as_str()),
            ("laptop", "XPS 13 9310")
        );
        assert_eq!(parse_windows("").form, "");
    }

    #[test]
    fn macs() {
        let k = dk::sanitize(mac_kind("MacBookPro18,3", None, None));
        assert_eq!(
            (k.form.as_str(), k.model.as_str(), k.icon.as_str()),
            ("laptop", "MacBook Pro", "mac_laptop")
        );
        let k = dk::sanitize(mac_kind("Mac14,3", Some("Mac mini"), Some(false)));
        assert_eq!(
            (k.form.as_str(), k.icon.as_str()),
            ("desktop", "mac_desktop")
        );
        let k = dk::sanitize(mac_kind("Mac15,4", Some("iMac"), Some(false)));
        assert_eq!(k.icon, "imac");
        let k = dk::sanitize(mac_kind("Mac14,2", None, Some(true)));
        assert_eq!(k.icon, "mac_laptop");
        assert_eq!(
            parse_model_name("Hardware:\n\n    Hardware Overview:\n\n      Model Name: MacBook Air\n      Model Identifier: Mac14,2\n"),
            Some("MacBook Air".into())
        );
        assert_eq!(parse_model_name("nothing"), None);
    }

    #[test]
    fn models() {
        assert_eq!(
            pick_model("Lenovo", "20XW004GUS", "ThinkPad X1"),
            "ThinkPad X1"
        );
        assert_eq!(
            pick_model("Dell Inc.", "OptiPlex 7090", "1.0"),
            "OptiPlex 7090"
        );
        assert_eq!(pick_model("", "To Be Filled By O.E.M.", ""), "");
        assert_eq!(useful_model("Default string"), None);
    }
}
