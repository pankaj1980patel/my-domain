// "Send with my-domain" from the OS file manager.
//
// Three ways in, one way through:
//  - the command line: `my-domain --send <path>...` (files and/or folders).
//    If my-domain is already running, the paths are handed to it and this
//    process exits; otherwise this one starts and handles them itself. The
//    Windows "Send to" shortcut and the Linux `.desktop` entries / Dolphin
//    service menu / Nautilus script all run exactly that;
//  - macOS Finder's Services menu ("Send with my-domain", declared under
//    `NSServices` in Info.plist): AppKit calls `sendWithMyDomain:userData:error:`
//    on the provider registered here, with the chosen items on a pasteboard;
//  - either way the paths land in `deliver`: the window comes forward and
//    the UI shows a "Send to…" picker (event `send-request`, then
//    `take_send_request`). Choosing a device runs `send_paths`: each file
//    goes through the core's `send_file`, each folder through `send_folder`,
//    and the device chosen is remembered and offered first next time.
//
// The handoff between two processes of the same user: the running app
// listens on 127.0.0.1 (a random port) and writes the port and a random
// token to `<config dir>/send-handoff.json` (owner-only). A second process
// sends one line of JSON, `{"v":1,"token":"…","paths":[…]}`, and gets `ok`
// back. Nothing is sent anywhere until you pick a device in the window, so
// at worst a stray local caller can open the picker.
//
// Windows and Linux add their menu entries from Settings ("Send from the
// file manager"); macOS lists the Services item by itself.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use mdcore::kv::FileKv;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::Eng;

/// Must match `identifier` in tauri.conf.json (a test checks): the handoff
/// file is found before Tauri starts, so the config dir is worked out here.
const IDENTIFIER: &str = "com.landiscover.app";
/// The handoff message's version.
const VERSION: u32 = 1;
/// Largest handoff message accepted (a few thousand long paths).
const MAX_MESSAGE: u64 = 1 << 20;
const LAST_KEY: &str = "send_to_last";
/// Emitted when paths arrive; the UI then calls `take_send_request`.
const EVENT: &str = "send-request";

// ---------------------------------------------------------------------------
// The command line
// ---------------------------------------------------------------------------

/// Why `--send` couldn't be used.
#[derive(Debug, PartialEq)]
pub enum ArgError {
    /// `--send` with nothing after it.
    NoPaths,
}

/// The paths after `--send`, if it's there. Everything after it is a path
/// (a lone `--` right after it is skipped), so names with spaces arrive as
/// the shell or file manager split them. macOS's old `-psn_…` launch
/// argument is ignored.
pub fn parse_args<I, S>(args: I) -> Result<Option<Vec<PathBuf>>, ArgError>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    let mut args = args.into_iter().map(Into::into).skip(1);
    if !args.by_ref().any(|a| a == "--send") {
        return Ok(None);
    }
    let paths: Vec<PathBuf> = args
        .enumerate()
        .filter(|(i, a)| !(*i == 0 && a == "--"))
        .map(|(_, a)| a)
        .filter(|a| !a.to_string_lossy().starts_with("-psn_"))
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        return Err(ArgError::NoPaths);
    }
    Ok(Some(paths))
}

/// Absolute paths of the ones that exist (relative ones are taken from
/// `cwd`: the running app has another working directory), and the ones
/// that don't.
pub fn resolve(paths: &[PathBuf], cwd: &Path) -> (Vec<String>, Vec<String>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for p in paths {
        let abs = if p.is_absolute() {
            p.clone()
        } else {
            cwd.join(p)
        };
        if abs.exists() {
            let abs = std::fs::canonicalize(&abs).unwrap_or(abs);
            let s = abs.to_string_lossy().into_owned();
            if !found.contains(&s) {
                found.push(s);
            }
        } else {
            missing.push(p.to_string_lossy().into_owned());
        }
    }
    (found, missing)
}

/// This process's `--send` paths (absolute, existing), or `None` without
/// `--send`. Problems are printed; with nothing usable left it's an empty
/// list (the running app just comes forward).
pub fn from_command_line() -> Option<Vec<String>> {
    match parse_args(std::env::args_os()) {
        Ok(None) => None,
        Err(ArgError::NoPaths) => {
            eprintln!("[send] --send needs one or more files or folders");
            Some(Vec::new())
        }
        Ok(Some(paths)) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let (found, missing) = resolve(&paths, &cwd);
            for m in missing {
                eprintln!("[send] not found: {m}");
            }
            Some(found)
        }
    }
}

// ---------------------------------------------------------------------------
// The handoff to a running app
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Handoff {
    v: u32,
    token: String,
    paths: Vec<String>,
}

/// Where the running app is listening.
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Listening {
    port: u16,
    token: String,
}

/// One line of JSON (newlines in names are escaped, so the line holds).
fn encode(token: &str, paths: &[String]) -> String {
    let msg = Handoff {
        v: VERSION,
        token: token.to_string(),
        paths: paths.to_vec(),
    };
    let mut line = serde_json::to_string(&msg).unwrap_or_default();
    line.push('\n');
    line
}

/// The paths in a handoff line, if it's ours and carries the right token.
fn decode(line: &str, token: &str) -> Result<Vec<String>, &'static str> {
    let msg: Handoff = serde_json::from_str(line.trim_end()).map_err(|_| "not a handoff")?;
    if msg.v != VERSION {
        return Err("unknown version");
    }
    if !same(msg.token.as_bytes(), token.as_bytes()) {
        return Err("wrong token");
    }
    Ok(msg.paths)
}

/// Constant-time comparison.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The app's config dir, as Tauri works it out (`dirs::config_dir()` +
/// identifier), without Tauri: the handoff runs before it starts.
fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join(IDENTIFIER))
}

/// `<config dir>/send-handoff.json` (one per lab instance, `TEST=...`).
fn handoff_file() -> Option<PathBuf> {
    let name = match crate::test_suffix() {
        Some(s) => format!("send-handoff-{s}.json"),
        None => "send-handoff.json".to_string(),
    };
    Some(config_dir()?.join(name))
}

/// Give the paths to a running my-domain; false if none answered (then
/// this process starts and handles them itself).
pub fn hand_off(paths: &[String]) -> bool {
    let Some(file) = handoff_file() else {
        return false;
    };
    let Some(info) = std::fs::read_to_string(file)
        .ok()
        .and_then(|s| serde_json::from_str::<Listening>(&s).ok())
    else {
        return false;
    };
    match send_handoff(info.port, &info.token, paths) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[send] my-domain isn't answering ({e}); starting it");
            false
        }
    }
}

fn send_handoff(port: u16, token: &str, paths: &[String]) -> std::io::Result<()> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(encode(token, paths).as_bytes())?;
    let mut answer = String::new();
    BufReader::new(stream).take(16).read_line(&mut answer)?;
    if answer.trim_end() == "ok" {
        Ok(())
    } else {
        Err(std::io::Error::other("refused"))
    }
}

fn new_token() -> Option<String> {
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// Listen for other processes' `--send`, and say where in the handoff file.
fn listen(app: AppHandle) {
    let (Some(file), Some(token)) = (handoff_file(), new_token()) else {
        eprintln!("[send] no config dir or randomness: --send starts a new window");
        return;
    };
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[send] can't listen for --send: {e}");
            return;
        }
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else {
        return;
    };
    let info = Listening {
        port,
        token: token.clone(),
    };
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, serde_json::to_vec(&info).unwrap_or_default()))
        .and_then(|()| crate::owner_only(&file));
    if let Err(e) = written {
        eprintln!("[send] can't write {}: {e}", file.display());
        return;
    }
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Some(paths) = serve(stream, &token) {
                deliver(&app, paths);
            }
        }
    });
}

/// Read one handoff, answer it; its paths if it was good.
fn serve(stream: TcpStream, token: &str) -> Option<Vec<String>> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut writer = stream.try_clone().ok()?;
    let mut line = String::new();
    BufReader::new(stream)
        .take(MAX_MESSAGE)
        .read_line(&mut line)
        .ok()?;
    match decode(&line, token) {
        Ok(paths) => {
            let _ = writer.write_all(b"ok\n");
            Some(paths)
        }
        Err(e) => {
            eprintln!("[send] refused a handoff: {e}");
            let _ = writer.write_all(b"no\n");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// In the app
// ---------------------------------------------------------------------------

/// One thing to send.
#[derive(Serialize, Clone, Debug, PartialEq, specta::Type)]
pub struct SendItem {
    pub path: String,
    /// Its last path component.
    pub name: String,
    pub folder: bool,
    /// A saved web link (`.url`, `.webloc`, a `.desktop` link): its link,
    /// so the picker can offer to download the file on a device instead.
    pub link: Option<String>,
}

/// What the "Send to…" picker shows.
#[derive(Serialize, Clone, Debug, specta::Type)]
pub struct SendRequest {
    pub items: Vec<SendItem>,
    /// The device chosen last time: offered first.
    pub last_device: Option<String>,
}

/// How a send went.
#[derive(Serialize, Clone, Debug, Default, PartialEq, specta::Type)]
pub struct SendOutcome {
    /// Files on their way (a folder counts each file in it).
    pub files: u32,
    /// The names that couldn't be sent, with why.
    pub failed: Vec<String>,
}

/// Paths waiting for the picker, and the device chosen last.
pub struct SendTo {
    pending: Mutex<Vec<String>>,
    kv: Option<FileKv>,
}

static APP: OnceLock<AppHandle> = OnceLock::new();

fn item(path: &str) -> SendItem {
    let p = Path::new(path);
    SendItem {
        path: path.to_string(),
        name: p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        folder: p.is_dir(),
        link: shortcut_link(p),
    }
}

/// The web link a saved-link file holds: Windows' `.url` (`URL=`), a Mac's
/// `.webloc` (an XML property list) or a Linux `.desktop` link (`URL=`).
/// Small files only; http(s) links only.
pub fn shortcut_link(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if !matches!(ext.as_str(), "url" | "webloc" | "desktop") {
        return None;
    }
    if std::fs::metadata(path).ok()?.len() > 64 * 1024 {
        return None;
    }
    link_in_shortcut(&ext, &std::fs::read_to_string(path).ok()?)
}

/// `shortcut_link` on a file's text (`ext` in lower case).
pub fn link_in_shortcut(ext: &str, text: &str) -> Option<String> {
    let raw = if ext == "webloc" {
        let after = &text[text.find("<key>URL</key>")? + 14..];
        let start = after.find("<string>")? + 8;
        let end = after[start..].find("</string>")? + start;
        after[start..end]
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
    } else {
        text.lines().map(str::trim).find_map(|l| {
            let (k, v) = l.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("url")
                .then(|| v.trim().to_string())
        })?
    };
    mdcore::features::read_later::find_url(raw.trim())
}

/// Paths to send arrived (handoff, Services menu, this process's own
/// `--send`): bring the window forward and let the UI ask where to.
pub fn deliver(app: &AppHandle, paths: Vec<String>) {
    if let Some(state) = app.try_state::<SendTo>() {
        if let Ok(mut pending) = state.pending.lock() {
            for p in paths {
                if !pending.contains(&p) {
                    pending.push(p);
                }
            }
        }
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || crate::tray::show_window(&handle));
    let _ = app.emit(EVENT, ());
}

/// Startup: keep this process's own `--send` paths, listen for others',
/// register the Services provider (macOS) and refresh the file manager
/// entries (they point at this executable, which may have moved).
pub fn init(app: &AppHandle, initial: Option<Vec<String>>) {
    let _ = APP.set(app.clone());
    let kv = app
        .path()
        .app_config_dir()
        .ok()
        .map(|dir| FileKv::new(dir.join("prefs.json")));
    app.manage(SendTo {
        pending: Mutex::new(initial.unwrap_or_default()),
        kv,
    });
    listen(app.clone());
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(mac::register);
    }
    #[cfg(any(windows, target_os = "linux"))]
    std::thread::spawn(|| {
        if menu::installed() {
            if let Err(e) = menu::install() {
                eprintln!("[send] couldn't refresh the file manager entry: {e}");
            }
        }
    });
}

/// The picker's paths (taken: asked again, it's `None` until more arrive).
#[tauri::command]
#[specta::specta]
pub fn take_send_request(state: State<SendTo>) -> Option<SendRequest> {
    let paths = std::mem::take(&mut *state.pending.lock().ok()?);
    if paths.is_empty() {
        return None;
    }
    let items = paths
        .iter()
        .map(|p| item(p))
        .filter(|i| Path::new(&i.path).exists())
        .collect::<Vec<_>>();
    if items.is_empty() {
        return None;
    }
    Some(SendRequest {
        items,
        last_device: state.kv.as_ref().and_then(|kv| kv.get(LAST_KEY)),
    })
}

/// Send the files and folders to a device; remembers it for next time.
#[tauri::command]
#[specta::specta]
pub fn send_paths(
    node_id: String,
    paths: Vec<String>,
    app: AppHandle,
    engine: State<Eng>,
    state: State<SendTo>,
) -> SendOutcome {
    let mut out = SendOutcome::default();
    for path in &paths {
        let it = item(path);
        let sent = if it.folder {
            engine.send_folder(&node_id, path)
        } else {
            engine.send_file(&node_id, path).map(|_| 1)
        };
        match sent {
            Ok(n) => out.files += n,
            Err(e) => out.failed.push(format!("{}: {e}", it.name)),
        }
    }
    if out.files > 0 {
        if let Some(kv) = &state.kv {
            if let Err(e) = kv.set(LAST_KEY, &node_id) {
                eprintln!("[send] couldn't remember the device: {e}");
            }
        }
    }
    let name = crate::notify::device_name(&app, &node_id);
    crate::notify::post_if_away(&app, "Files", &summary(&out, &name));
    out
}

/// "Sending 3 files to Pixel" / "… 1 couldn't be sent."
fn summary(out: &SendOutcome, device: &str) -> String {
    let mut s = match out.files {
        0 => format!("Nothing was sent to {device}."),
        1 => format!("Sending a file to {device}."),
        n => format!("Sending {n} files to {device}."),
    };
    match out.failed.len() {
        0 => {}
        1 => s.push_str(" 1 couldn't be sent."),
        n => s.push_str(&format!(" {n} couldn't be sent.")),
    }
    s
}

/// The "Send from the file manager" setting.
#[derive(Serialize, Clone, Debug, specta::Type)]
pub struct FileManagerMenu {
    /// "macos", "windows" or "linux".
    pub os: String,
    /// Whether there's a switch (macOS's Services item is always there).
    pub switchable: bool,
    pub on: bool,
}

fn menu_state() -> FileManagerMenu {
    #[cfg(any(windows, target_os = "linux"))]
    let (switchable, on) = (true, menu::installed());
    #[cfg(not(any(windows, target_os = "linux")))]
    let (switchable, on) = (false, cfg!(target_os = "macos"));
    FileManagerMenu {
        os: std::env::consts::OS.to_string(),
        switchable,
        on,
    }
}

#[tauri::command]
#[specta::specta]
pub fn file_manager_menu() -> FileManagerMenu {
    menu_state()
}

/// Add or remove "Send with my-domain" in Explorer's Send to menu
/// (Windows) or the file manager's menus (Linux).
#[tauri::command]
#[specta::specta]
pub fn set_file_manager_menu(on: bool) -> Result<FileManagerMenu, mdcore::CoreError> {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let done = if on { menu::install() } else { menu::remove() };
        done.map_err(mdcore::CoreError::Other)?;
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    let _ = on;
    Ok(menu_state())
}

/// The executable the menu entries run (an AppImage's own path on Linux:
/// the running binary lives in a temporary mount).
#[cfg(any(windows, target_os = "linux"))]
fn exe() -> Result<PathBuf, String> {
    if cfg!(target_os = "linux") {
        if let Some(app_image) = std::env::var_os("APPIMAGE") {
            return Ok(PathBuf::from(app_image));
        }
    }
    std::env::current_exe().map_err(|e| format!("can't find my-domain's executable: {e}"))
}

// ---------------------------------------------------------------------------
// Windows: a shortcut in Explorer's "Send to" folder
// ---------------------------------------------------------------------------

/// PowerShell that writes the shortcut; the paths come in the environment
/// (`MD_LNK`, `MD_EXE`), so nothing in them needs quoting.
#[cfg(any(windows, test))]
const LNK_SCRIPT: &str = "$s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:MD_LNK);\
$s.TargetPath=$env:MD_EXE;$s.Arguments='--send';$s.IconLocation=$env:MD_EXE+',0';\
$s.Description='Send with my-domain';$s.Save()";

#[cfg(windows)]
mod menu {
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;

    /// No console window flashing up.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// `%APPDATA%\Microsoft\Windows\SendTo\my-domain.lnk`.
    fn link() -> Option<PathBuf> {
        std::env::var_os("APPDATA")
            .map(|a| PathBuf::from(a).join(r"Microsoft\Windows\SendTo\my-domain.lnk"))
    }

    pub fn installed() -> bool {
        link().is_some_and(|l| l.exists())
    }

    pub fn install() -> Result<(), String> {
        let lnk = link().ok_or("no Send to folder")?;
        let exe = super::exe()?;
        let status = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                super::LNK_SCRIPT,
            ])
            .env("MD_LNK", &lnk)
            .env("MD_EXE", &exe)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("couldn't run PowerShell: {e}"))?;
        if status.success() && lnk.exists() {
            Ok(())
        } else {
            Err("couldn't add the Send to shortcut".into())
        }
    }

    pub fn remove() -> Result<(), String> {
        match link() {
            Some(l) if l.exists() => std::fs::remove_file(l).map_err(|e| e.to_string()),
            _ => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Linux: an "Open with" entry, a Dolphin service menu, a Nautilus script
// ---------------------------------------------------------------------------

/// An argument for a `.desktop` file's `Exec=`: quoted, with the quoting
/// rules of the Desktop Entry spec, then escaped as a string value.
#[cfg(any(target_os = "linux", test))]
fn desktop_arg(arg: &str) -> String {
    let mut q = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                q.push('\\');
                q.push(c);
            }
            '%' => q.push_str("%%"),
            _ => q.push(c),
        }
    }
    q.push('"');
    // A string value's own escape: a backslash is written twice.
    q.replace('\\', "\\\\")
}

/// `~/.local/share/applications/my-domain-send.desktop`: lists my-domain in
/// "Open with" for any file or folder, without showing in the app menu.
#[cfg(any(target_os = "linux", test))]
fn open_with_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Send with my-domain\n\
         Comment=Send files and folders to one of your devices\n\
         Exec={} --send %F\n\
         Icon=document-send\n\
         Terminal=false\n\
         NoDisplay=true\n\
         MimeType=application/octet-stream;inode/directory;\n\
         X-my-domain=send\n",
        desktop_arg(exe)
    )
}

/// `~/.local/share/kio/servicemenus/my-domain-send.desktop`: Dolphin's
/// right-click menu.
#[cfg(any(target_os = "linux", test))]
fn dolphin_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Service\n\
         X-KDE-ServiceTypes=KonqPopupMenu/Plugin\n\
         MimeType=all/all;\n\
         Actions=sendWithMyDomain;\n\
         \n\
         [Desktop Action sendWithMyDomain]\n\
         Name=Send with my-domain\n\
         Icon=document-send\n\
         Exec={} --send %F\n",
        desktop_arg(exe)
    )
}

/// `~/.local/share/nautilus/scripts/Send with my-domain`: Files' (Nautilus)
/// right-click Scripts menu. Nautilus lists the chosen paths one per line.
#[cfg(any(target_os = "linux", test))]
fn nautilus_script(exe: &str) -> String {
    let quoted = format!("'{}'", exe.replace('\'', r"'\''"));
    format!(
        "#!/bin/sh\n\
         # Added by my-domain (Settings: Send from the file manager).\n\
         set -f\n\
         IFS='\n'\n\
         exec {quoted} --send $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n"
    )
}

#[cfg(target_os = "linux")]
mod menu {
    use std::path::PathBuf;

    fn data_home() -> Option<PathBuf> {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }

    /// One menu entry: (file, contents maker, executable).
    type Entry = (PathBuf, fn(&str) -> String, bool);

    fn files() -> Option<[Entry; 3]> {
        let d = data_home()?;
        Some([
            (
                d.join("applications/my-domain-send.desktop"),
                super::open_with_entry,
                false,
            ),
            (
                d.join("kio/servicemenus/my-domain-send.desktop"),
                super::dolphin_entry,
                true,
            ),
            (
                d.join("nautilus/scripts/Send with my-domain"),
                super::nautilus_script,
                true,
            ),
        ])
    }

    pub fn installed() -> bool {
        files().is_some_and(|f| f[0].0.exists())
    }

    pub fn install() -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let exe = super::exe()?;
        let exe = exe.to_string_lossy();
        let files = files().ok_or("no home folder")?;
        for (path, make, executable) in files {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, make(&exe)).map_err(|e| format!("{}: {e}", path.display()))?;
            let mode = if executable { 0o755 } else { 0o644 };
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode));
        }
        refresh_mime_cache();
        Ok(())
    }

    pub fn remove() -> Result<(), String> {
        for (path, _, _) in files().ok_or("no home folder")? {
            if path.exists() {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        refresh_mime_cache();
        Ok(())
    }

    /// So "Open with" sees the change now (skipped where it's not installed).
    fn refresh_mime_cache() {
        if let Some(dir) = data_home().map(|d| d.join("applications")) {
            let _ = std::process::Command::new("update-desktop-database")
                .arg(dir)
                .status();
        }
    }
}

// ---------------------------------------------------------------------------
// macOS: Finder's Services menu
// ---------------------------------------------------------------------------

/// The Services menu sent these (main thread).
#[cfg(target_os = "macos")]
fn from_service(paths: Vec<String>) {
    if let Some(app) = APP.get() {
        if paths.is_empty() {
            eprintln!("[send] the Services menu gave no files");
        }
        deliver(app, paths);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, NSObject};
    use objc2::{define_class, msg_send, AllocAnyThread};
    use objc2_foundation::{NSArray, NSString};

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {
        /// Re-reads the apps' NSServices so the menu item shows up without
        /// logging out (where the system allows it).
        fn NSUpdateDynamicServices();
    }

    define_class!(
        /// The provider for Info.plist's `NSServices` entry
        /// (`NSMessage` = `sendWithMyDomain`).
        #[unsafe(super(NSObject))]
        #[name = "MyDomainSendService"]
        struct SendService;

        impl SendService {
            #[unsafe(method(sendWithMyDomain:userData:error:))]
            fn send_with_my_domain(
                &self,
                pboard: &AnyObject,
                _user_data: *mut AnyObject,
                _error: *mut *mut AnyObject,
            ) {
                // SAFETY: AppKit hands us an NSPasteboard, on the main thread.
                let paths = unsafe { file_paths(pboard) };
                super::from_service(paths);
            }
        }
    );

    impl SendService {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            // SAFETY: NSObject's plain init.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The file URLs on the pasteboard, as paths.
    unsafe fn file_paths(pboard: &AnyObject) -> Vec<String> {
        let Some(url_class) = AnyClass::get(c"NSURL") else {
            return Vec::new();
        };
        // A class is an object too: `readObjectsForClasses:` takes them.
        let class_obj: &AnyObject = &*(url_class as *const AnyClass as *const AnyObject);
        let classes = NSArray::from_slice(&[class_obj]);
        let urls: Option<Retained<NSArray<AnyObject>>> = msg_send![
            pboard,
            readObjectsForClasses: &*classes,
            options: std::ptr::null::<AnyObject>()
        ];
        let Some(urls) = urls else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for i in 0..urls.count() {
            let url = urls.objectAtIndex(i);
            let is_file: bool = msg_send![&*url, isFileURL];
            if !is_file {
                continue;
            }
            let path: Option<Retained<NSString>> = msg_send![&*url, path];
            if let Some(path) = path {
                out.push(path.to_string());
            }
        }
        out
    }

    /// Make this app the Services provider (main thread).
    pub fn register() {
        let Some(app_class) = AnyClass::get(c"NSApplication") else {
            return;
        };
        let provider = SendService::new();
        // SAFETY: main thread; NSApplication exists once Tauri is running.
        unsafe {
            let app: Option<Retained<AnyObject>> = msg_send![app_class, sharedApplication];
            let Some(app) = app else {
                return;
            };
            let _: () = msg_send![&*app, setServicesProvider: &*provider];
            NSUpdateDynamicServices();
        }
        // AppKit holds the provider weakly: it lives as long as the app.
        std::mem::forget(provider);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_links_are_read() {
        assert_eq!(
            link_in_shortcut(
                "url",
                "[InternetShortcut]\r\nURL=https://example.com/a.zip\r\n"
            )
            .as_deref(),
            Some("https://example.com/a.zip")
        );
        let webloc = "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>URL</key><string>https://x.io/f?a=1&amp;b=2</string></dict></plist>";
        assert_eq!(
            link_in_shortcut("webloc", webloc).as_deref(),
            Some("https://x.io/f?a=1&b=2")
        );
        assert_eq!(
            link_in_shortcut(
                "desktop",
                "[Desktop Entry]\nType=Link\nURL=http://x.io/r.pdf\n"
            )
            .as_deref(),
            Some("http://x.io/r.pdf")
        );
        assert_eq!(link_in_shortcut("url", "URL=file:///etc/passwd"), None);
        assert_eq!(link_in_shortcut("webloc", "bplist00"), None);
    }

    fn args(v: &[&str]) -> Vec<String> {
        std::iter::once("my-domain")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn without_send_there_is_nothing_to_do() {
        assert_eq!(parse_args(args(&[])), Ok(None));
        assert_eq!(parse_args(args(&["/tmp/a.txt"])), Ok(None));
    }

    #[test]
    fn paths_after_send_keep_their_spaces() {
        let got = parse_args(args(&["--send", "/Users/me/My Photos", "/tmp/a b.txt"]));
        assert_eq!(
            got,
            Ok(Some(vec![
                PathBuf::from("/Users/me/My Photos"),
                PathBuf::from("/tmp/a b.txt")
            ]))
        );
    }

    #[test]
    fn a_lone_double_dash_and_psn_are_skipped() {
        let got = parse_args(args(&[
            "-psn_0_1234",
            "--send",
            "--",
            "-odd name",
            "-psn_0_9",
        ]));
        assert_eq!(got, Ok(Some(vec![PathBuf::from("-odd name")])));
    }

    #[test]
    fn send_without_paths_is_an_error() {
        assert_eq!(parse_args(args(&["--send"])), Err(ArgError::NoPaths));
        assert_eq!(parse_args(args(&["--send", "--"])), Err(ArgError::NoPaths));
    }

    #[test]
    fn mixed_files_and_folders_resolve_and_missing_ones_are_reported() {
        let dir = std::env::temp_dir().join(format!("md-send-{}", std::process::id()));
        let folder = dir.join("a folder");
        std::fs::create_dir_all(&folder).unwrap();
        let file = dir.join("one file.txt");
        std::fs::write(&file, b"x").unwrap();
        let (found, missing) = resolve(
            &[
                PathBuf::from("one file.txt"),
                folder.clone(),
                PathBuf::from("nope.txt"),
                file.clone(),
            ],
            &dir,
        );
        let canon = |p: &Path| {
            std::fs::canonicalize(p)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        // The relative one is the same file as the absolute one: listed once.
        assert_eq!(found, vec![canon(&file), canon(&folder)]);
        assert_eq!(missing, vec!["nope.txt".to_string()]);
        let items: Vec<SendItem> = found.iter().map(|p| item(p)).collect();
        assert!(!items[0].folder && items[0].name == "one file.txt");
        assert!(items[1].folder && items[1].name == "a folder");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn handoff_is_one_json_line_with_the_token() {
        let paths = vec!["/a b/c.txt".to_string(), "/odd\nname".to_string()];
        let line = encode("tok", &paths);
        assert!(line.ends_with('\n'));
        assert_eq!(
            line.matches('\n').count(),
            1,
            "newlines in names are escaped"
        );
        assert_eq!(
            line,
            "{\"v\":1,\"token\":\"tok\",\"paths\":[\"/a b/c.txt\",\"/odd\\nname\"]}\n"
        );
        assert_eq!(decode(&line, "tok"), Ok(paths));
    }

    #[test]
    fn handoff_refuses_a_wrong_token_or_version_or_junk() {
        let line = encode("tok", &["/x".to_string()]);
        assert_eq!(decode(&line, "other"), Err("wrong token"));
        assert_eq!(
            decode("{\"v\":2,\"token\":\"tok\",\"paths\":[]}", "tok"),
            Err("unknown version")
        );
        assert_eq!(decode("GET / HTTP/1.1", "tok"), Err("not a handoff"));
    }

    #[test]
    fn handoff_round_trips_over_a_socket() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let first = serve(s, "secret");
            let (s, _) = listener.accept().unwrap();
            (first, serve(s, "secret"))
        });
        let paths = vec!["/tmp/with space".to_string()];
        assert!(send_handoff(port, "secret", &paths).is_ok());
        assert!(send_handoff(port, "guess", &paths).is_err());
        let (first, second) = server.join().unwrap();
        assert_eq!(first, Some(paths));
        assert_eq!(second, None);
    }

    #[test]
    fn identifier_matches_tauri_conf() {
        let conf = include_str!("../tauri.conf.json");
        let v: serde_json::Value = serde_json::from_str(conf).unwrap();
        assert_eq!(v["identifier"], IDENTIFIER);
    }

    #[test]
    fn summary_reads_well() {
        let o = |files, failed: &[&str]| SendOutcome {
            files,
            failed: failed.iter().map(|s| s.to_string()).collect(),
        };
        assert_eq!(summary(&o(1, &[]), "Pixel"), "Sending a file to Pixel.");
        assert_eq!(
            summary(&o(3, &["x: gone"]), "Pixel"),
            "Sending 3 files to Pixel. 1 couldn't be sent."
        );
        assert_eq!(
            summary(&o(0, &["a", "b"]), "Pixel"),
            "Nothing was sent to Pixel. 2 couldn't be sent."
        );
    }

    #[test]
    fn desktop_exec_quotes_spaces_and_specials() {
        assert_eq!(desktop_arg("/opt/my domain/bin"), "\"/opt/my domain/bin\"");
        // `$` and `"` get a backslash, which the string value doubles.
        assert_eq!(desktop_arg("/a$b\"c"), "\"/a\\\\$b\\\\\"c\"");
        assert_eq!(desktop_arg("/100%"), "\"/100%%\"");
        let entry = open_with_entry("/opt/my domain/my-domain");
        assert!(entry.contains("Exec=\"/opt/my domain/my-domain\" --send %F\n"));
        assert!(entry.contains("MimeType=application/octet-stream;inode/directory;\n"));
        assert!(dolphin_entry("/x").contains("[Desktop Action sendWithMyDomain]\n"));
    }

    #[test]
    fn nautilus_script_quotes_the_executable() {
        let s = nautilus_script("/home/me/it's here/my-domain");
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains(
            "exec '/home/me/it'\\''s here/my-domain' --send $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\n"
        ));
    }

    #[test]
    fn windows_shortcut_passes_send() {
        assert!(LNK_SCRIPT.contains("$s.Arguments='--send'"));
        assert!(LNK_SCRIPT.contains("$env:MD_EXE"));
    }
}
