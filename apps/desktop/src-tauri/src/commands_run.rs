// Run command: named shell commands this computer's owner writes in Settings
// ("Commands my other devices can run"), shown as buttons on your other
// devices, which can run them.
//
// This computer stays in charge:
//  - only commands written here can run: a device sends a command's id (the
//    `run` action, `{"id": …}`), never command text or arguments; an unknown
//    or switched-off id is refused;
//  - nothing until you add one (`<app config dir>/commands.json`, key
//    `commands`); each has its own on/off switch;
//  - a notification each time one runs, saying which device ran it;
//  - as you, never elevated: `sh -c` (`cmd /C` on Windows), no input, in
//    its own process group so a timeout kills everything it started;
//  - stopped after `TIMEOUT`; the exit code and the last `MAX_OUTPUT` bytes
//    of what it printed go back in the status (`last_run`), which the device
//    asks for again.
//
// The status lists `run` in `actions` and `commands: [{id, name}]` while at
// least one command is on.

use std::collections::BTreeSet;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mdcore::kv::FileKv;
use mdcore::CoreError;
use tauri::{AppHandle, Manager, State};

const KEY: &str = "commands";
/// The most commands kept.
const MAX_COMMANDS: usize = 50;
/// The longest name, in characters.
const MAX_NAME: usize = 60;
/// The longest command line, in bytes.
const MAX_LINE: usize = 2000;
/// How long a command may run before it's stopped.
const TIMEOUT: Duration = Duration::from_secs(60);
/// How much of the end of its output is kept, in bytes.
pub const MAX_OUTPUT: usize = 2048;

/// One command your other devices can run.
#[derive(serde::Serialize, serde::Deserialize, specta::Type, Clone, Debug, PartialEq)]
pub struct ShellCommand {
    pub id: String,
    /// What the button says ("Back up photos").
    pub name: String,
    /// The command line, run with `sh -c`.
    pub command: String,
    pub enabled: bool,
}

/// A new command, from the UI.
#[derive(serde::Deserialize, specta::Type, Clone, Debug)]
pub struct NewShellCommand {
    pub name: String,
    pub command: String,
    pub enabled: bool,
}

/// How a run went.
#[derive(serde::Serialize, specta::Type, Clone, Debug, PartialEq)]
pub struct RunOutcome {
    /// Its exit code; none when it was stopped (or killed by a signal).
    pub code: Option<i32>,
    /// The last `MAX_OUTPUT` bytes of what it printed (output and errors).
    pub output: String,
    /// It ran past the time limit and was stopped.
    pub timed_out: bool,
}

/// The command a device may run by `id`: refused when there's none, or
/// it's switched off.
pub fn find<'a>(list: &'a [ShellCommand], id: &str) -> Result<&'a ShellCommand, String> {
    let c = list
        .iter()
        .find(|c| c.id == id)
        .ok_or("there's no such command on this computer")?;
    if !c.enabled {
        return Err(format!(
            "\u{201c}{}\u{201d} is turned off on this computer",
            c.name
        ));
    }
    Ok(c)
}

/// The last `max` bytes of `bytes` as text, cut on a character boundary.
pub fn tail(bytes: &[u8], max: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.len() <= max {
        return text.into_owned();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// Run `line` in the shell, as this user, for at most `timeout`; output and
/// errors together.
pub fn exec(line: &str, timeout: Duration) -> RunOutcome {
    match spawn_and_wait(line, timeout) {
        Ok(out) => out,
        Err(e) => RunOutcome {
            code: None,
            output: format!("couldn't start it: {e}"),
            timed_out: false,
        },
    }
}

fn spawn_and_wait(line: &str, timeout: Duration) -> std::io::Result<RunOutcome> {
    let (mut reader, writer) = std::io::pipe()?;
    let mut child = {
        // The command is dropped at the end of this block, so the pipe
        // closes once the child (and anything it started) is gone.
        let mut cmd = shell(line);
        cmd.stdin(Stdio::null())
            .stdout(writer.try_clone()?)
            .stderr(writer);
        cmd.spawn()?
    };
    // Read as it runs (a full pipe would stall it), keeping only the end.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            kept.extend_from_slice(&buf[..n]);
            if kept.len() > MAX_OUTPUT * 2 {
                kept.drain(..kept.len() - MAX_OUTPUT * 2);
            }
        }
        let _ = tx.send(kept);
    });
    let start = Instant::now();
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (Some(status), false);
        }
        if start.elapsed() >= timeout {
            kill(&mut child);
            break (child.wait().ok(), true);
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // Something it left running in the background may still hold the
    // output open: don't wait for it long.
    let kept = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
    Ok(RunOutcome {
        code: if timed_out {
            None
        } else {
            status.and_then(|s| s.code())
        },
        output: tail(&kept, MAX_OUTPUT),
        timed_out,
    })
}

#[cfg(unix)]
fn shell(line: &str) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(line).process_group(0);
    cmd
}

#[cfg(windows)]
fn shell(line: &str) -> Command {
    let mut cmd = Command::new("cmd");
    cmd.arg("/C").arg(line);
    cmd
}

/// Stop it and everything it started (its process group).
#[cfg(unix)]
fn kill(child: &mut std::process::Child) {
    let group = format!("-{}", child.id());
    let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
    let _ = child.kill();
}

#[cfg(windows)]
fn kill(child: &mut std::process::Child) {
    let pid = child.id().to_string();
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid])
        .status();
    let _ = child.kill();
}

/// Check what the UI sent, and tidy it.
fn checked(mut c: ShellCommand) -> Result<ShellCommand, CoreError> {
    let bad = |why: &str| Err(CoreError::InvalidInput(why.to_string()));
    c.name = c.name.trim().chars().take(MAX_NAME).collect();
    c.command = c.command.trim().to_string();
    if c.name.is_empty() {
        return bad("give it a name");
    }
    if c.command.is_empty() {
        return bad("write the command to run");
    }
    if c.command.len() > MAX_LINE {
        return bad("that command is too long");
    }
    Ok(c)
}

/// The commands, where they're saved, the last run and what's running now.
pub struct ShellCommands {
    kv: Option<FileKv>,
    list: Mutex<Vec<ShellCommand>>,
    /// The last run a device asked for, for the status (`last_run`).
    last: Mutex<Option<serde_json::Value>>,
    /// Ids running now: one run of each at a time.
    running: Mutex<BTreeSet<String>>,
}

impl ShellCommands {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("commands.json")));
        let list = kv
            .as_ref()
            .and_then(|kv| kv.get(KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        ShellCommands {
            kv,
            list: Mutex::new(list),
            last: Mutex::new(None),
            running: Mutex::new(BTreeSet::new()),
        }
    }

    fn save(&self, list: &[ShellCommand]) {
        let (Some(kv), Ok(json)) = (&self.kv, serde_json::to_string(list)) else {
            return;
        };
        if let Err(e) = kv.set(KEY, &json) {
            eprintln!("[commands] couldn't save the commands: {e}");
        }
    }

    fn list(&self) -> Vec<ShellCommand> {
        self.list.lock().map(|l| l.clone()).unwrap_or_default()
    }

    fn add(&self, new: NewShellCommand) -> Result<ShellCommand, CoreError> {
        let c = checked(ShellCommand {
            id: uuid::Uuid::new_v4().to_string(),
            name: new.name,
            command: new.command,
            enabled: new.enabled,
        })?;
        let mut list = self.list.lock().map_err(|_| busy())?;
        if list.len() >= MAX_COMMANDS {
            return Err(CoreError::InvalidInput(format!(
                "at most {MAX_COMMANDS} commands"
            )));
        }
        list.push(c.clone());
        self.save(&list);
        Ok(c)
    }

    fn update(&self, c: ShellCommand) -> Result<ShellCommand, CoreError> {
        let c = checked(c)?;
        let mut list = self.list.lock().map_err(|_| busy())?;
        let old = list.iter_mut().find(|x| x.id == c.id).ok_or_else(gone)?;
        *old = c.clone();
        self.save(&list);
        Ok(c)
    }

    fn delete(&self, id: &str) {
        let Ok(mut list) = self.list.lock() else {
            return;
        };
        let before = list.len();
        list.retain(|c| c.id != id);
        if list.len() != before {
            self.save(&list);
        }
    }

    fn get(&self, id: &str) -> Option<ShellCommand> {
        self.list.lock().ok()?.iter().find(|c| c.id == id).cloned()
    }

    /// The command a device may run by `id` (see `find`).
    fn runnable(&self, id: &str) -> Result<ShellCommand, String> {
        let list = self.list.lock().map_err(|_| "commands unavailable")?;
        find(&list, id).cloned()
    }

    fn set_last(&self, last: serde_json::Value) {
        if let Ok(mut l) = self.last.lock() {
            *l = Some(last);
        }
    }
}

fn busy() -> CoreError {
    CoreError::Other("commands unavailable".into())
}

fn gone() -> CoreError {
    CoreError::InvalidInput("that command is gone".into())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `run` in the actions and the commands that are on, with the last run,
/// into the status.
pub fn extend(app: &AppHandle, status: &mut serde_json::Value, actions: &mut Vec<&str>) {
    let Some(state) = app.try_state::<ShellCommands>() else {
        return;
    };
    let on: Vec<serde_json::Value> = state
        .list()
        .into_iter()
        .filter(|c| c.enabled)
        .map(|c| serde_json::json!({ "id": c.id, "name": c.name }))
        .collect();
    if on.is_empty() {
        return;
    }
    actions.push("run");
    status["commands"] = on.into();
    if let Some(last) = state.last.lock().ok().and_then(|l| l.clone()) {
        status["last_run"] = last;
    }
}

/// A device (`from`) asked to run command `{id}`: run it off this thread,
/// keep how it went for the status, and call `done` (to push the status).
pub fn on_run(
    app: &AppHandle,
    from: &str,
    args: &serde_json::Value,
    done: impl FnOnce(&AppHandle) + Send + 'static,
) {
    let Some(state) = app.try_state::<ShellCommands>() else {
        return;
    };
    let id = args["id"].as_str().unwrap_or_default().to_string();
    let c = match state.runnable(&id) {
        Ok(c) => c,
        Err(why) => {
            eprintln!("[commands] refused {id:?} from {from}: {why}");
            state.set_last(serde_json::json!({
                "id": id, "name": "", "code": null, "output": "",
                "error": why, "at": now_secs(),
            }));
            done(app);
            return;
        }
    };
    let fresh = state
        .running
        .lock()
        .map(|mut r| r.insert(c.id.clone()))
        .unwrap_or(false);
    if !fresh {
        eprintln!("[commands] {:?} is still running", c.name);
        return;
    }
    let device = who(app, from);
    crate::notify::post_always(
        app,
        &format!("{device} ran \u{201c}{}\u{201d}", c.name),
        "Change which commands your devices can run in my-domain\u{2019}s Settings.",
    );
    let app = app.clone();
    std::thread::spawn(move || {
        let out = exec(&c.command, TIMEOUT);
        if let Some(state) = app.try_state::<ShellCommands>() {
            state.set_last(serde_json::json!({
                "id": c.id, "name": c.name, "code": out.code, "output": out.output,
                "timed_out": out.timed_out, "at": now_secs(),
            }));
            if let Ok(mut r) = state.running.lock() {
                r.remove(&c.id);
            }
        }
        done(&app);
    });
}

/// The device's name for the notification ("This computer" for a schedule
/// here).
fn who(app: &AppHandle, from: &str) -> String {
    let me = app
        .try_state::<crate::Eng>()
        .is_some_and(|e| e.identity().node_id == from);
    if me {
        "This computer".into()
    } else {
        crate::notify::device_name(app, from)
    }
}

/// The commands your other devices can run.
#[tauri::command]
#[specta::specta]
pub fn list_shell_commands(state: State<ShellCommands>) -> Vec<ShellCommand> {
    state.list()
}

/// Add a command your other devices can run.
#[tauri::command]
#[specta::specta]
pub fn add_shell_command(
    command: NewShellCommand,
    app: AppHandle,
    state: State<ShellCommands>,
) -> Result<ShellCommand, CoreError> {
    let c = state.add(command)?;
    refresh(&app);
    Ok(c)
}

/// Change a command (its name, command line, on/off).
#[tauri::command]
#[specta::specta]
pub fn update_shell_command(
    command: ShellCommand,
    app: AppHandle,
    state: State<ShellCommands>,
) -> Result<ShellCommand, CoreError> {
    let c = state.update(command)?;
    refresh(&app);
    Ok(c)
}

/// Remove a command.
#[tauri::command]
#[specta::specta]
pub fn delete_shell_command(id: String, app: AppHandle, state: State<ShellCommands>) {
    state.delete(&id);
    refresh(&app);
}

/// Run a command here, to try it (on or off; no notification). Waits for it
/// to finish, up to the time limit.
#[tauri::command]
#[specta::specta]
pub async fn test_shell_command(id: String, app: AppHandle) -> Result<RunOutcome, CoreError> {
    let c = app
        .try_state::<ShellCommands>()
        .and_then(|s| s.get(&id))
        .ok_or_else(gone)?;
    tauri::async_runtime::spawn_blocking(move || exec(&c.command, TIMEOUT))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))
}

/// The list changed: tell the core, off the UI's thread.
fn refresh(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || crate::device::refresh_status(&app));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(id: &str, enabled: bool) -> ShellCommand {
        ShellCommand {
            id: id.into(),
            name: format!("Command {id}"),
            command: "true".into(),
            enabled,
        }
    }

    #[test]
    fn only_known_commands_that_are_on() {
        let list = vec![cmd("a", true), cmd("b", false)];
        assert_eq!(find(&list, "a").unwrap().id, "a");
        assert!(find(&list, "b").is_err(), "switched off");
        assert!(find(&list, "c").is_err(), "unknown");
        assert!(find(&list, "").is_err());
        assert!(find(&[], "a").is_err());
    }

    #[test]
    fn keeps_the_end_of_the_output() {
        assert_eq!(tail(b"hello", 10), "hello");
        assert_eq!(tail(b"hello world", 5), "world");
        let long = "x".repeat(5000) + "END";
        let t = tail(long.as_bytes(), MAX_OUTPUT);
        assert_eq!(t.len(), MAX_OUTPUT);
        assert!(t.ends_with("END"));
        // Never cuts a character in half: "é" is two bytes.
        assert_eq!(tail("aé".as_bytes(), 2), "é");
        assert_eq!(tail("éé".as_bytes(), 3), "é");
    }

    #[test]
    fn checks_what_the_ui_sends() {
        let mut c = cmd("a", true);
        c.name = "  Back up  ".into();
        c.command = "  echo hi \n".into();
        let ok = checked(c.clone()).unwrap();
        assert_eq!(
            (ok.name.as_str(), ok.command.as_str()),
            ("Back up", "echo hi")
        );
        c.name = " ".into();
        assert!(checked(c.clone()).is_err());
        c.name = "x".into();
        c.command = String::new();
        assert!(checked(c.clone()).is_err());
        c.command = "y".repeat(MAX_LINE + 1);
        assert!(checked(c).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn exit_code_and_output() {
        let out = exec("echo out; echo err >&2; exit 3", Duration::from_secs(10));
        assert_eq!(out.code, Some(3));
        assert!(!out.timed_out);
        assert!(out.output.contains("out"), "{:?}", out.output);
        assert!(out.output.contains("err"), "{:?}", out.output);
    }

    #[cfg(unix)]
    #[test]
    fn long_output_is_cut_to_the_end() {
        let out = exec(
            "i=0; while [ $i -lt 3000 ]; do echo line$i; i=$((i+1)); done",
            Duration::from_secs(10),
        );
        assert_eq!(out.code, Some(0));
        assert!(out.output.len() <= MAX_OUTPUT);
        assert!(out.output.ends_with("line2999\n"), "{:?}", out.output);
    }

    #[cfg(unix)]
    #[test]
    fn stopped_after_the_time_limit() {
        let start = Instant::now();
        // The sleep runs in the background too, holding the output open:
        // the whole group goes.
        let out = exec(
            "echo started; sleep 5 & sleep 5",
            Duration::from_millis(300),
        );
        assert!(out.timed_out);
        assert_eq!(out.code, None);
        assert!(out.output.contains("started"));
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{:?}",
            start.elapsed()
        );
    }
}
