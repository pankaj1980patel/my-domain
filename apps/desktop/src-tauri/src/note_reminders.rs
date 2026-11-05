// Reminders on notes (the core's `features::notes`, `remind`): a note can
// remind you at a time, on every device that has it, whichever is on. This
// is the computer's side: it shows the reminder here.
//
//  - a thread wakes at the next reminder's time (and at least every minute,
//    so a clock change or a sleep doesn't throw it off), and whenever the
//    notes change (`changed`: saved here, or arrived from another device);
//  - a reminder shows once: what was shown (note id -> its time) is kept in
//    `<app config dir>/note_reminders.json`, so a restart doesn't show it
//    again; a new time (snooze) is a new reminder;
//  - one that came due while the app was closed or the computer asleep is
//    shown once when the app is back, if it's less than `LATE_MS` (24 h)
//    late; an older one is skipped (counted as shown);
//  - it's an OS notification, "<title>" with the first line of the body,
//    and a `note-reminder` event for the window, which shows it with Open,
//    Done and Snooze (desktop notifications can't carry buttons on every
//    OS, and a click on one doesn't reach the app);
//  - Done (here or on any device) and a new time stop it; on this
//    computer the window's reminder goes when the note syncs Done (an OS
//    notification already shown can't be taken back here).

use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use mdcore::kv::FileKv;
use mdcore::model::Note;
use mdcore::CoreError;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::notes::NoteView;
use crate::Eng;

const KEY: &str = "fired";
/// A reminder this late (app closed, computer asleep) is skipped, not shown.
pub const LATE_MS: u64 = 24 * 3600 * 1000;
/// The longest the thread sleeps between looks.
const MAX_SLEEP_MS: u64 = 60_000;
/// The longest notification text from a note's body.
const MAX_LINE: usize = 200;

/// What to do with one note's reminder now.
#[derive(Debug, PartialEq)]
pub enum Due {
    /// No reminder, Done, or already shown.
    Nothing,
    /// Not yet: at this time (unix ms).
    Later(u64),
    /// Show it now.
    Show,
    /// Too late (over `LATE_MS`): count it as shown, say nothing.
    Skip,
}

/// One note's reminder at `now` (`shown`: the time it was last shown for).
pub fn due(at: Option<u64>, done: bool, shown: Option<u64>, now: u64) -> Due {
    let Some(at) = at else {
        return Due::Nothing;
    };
    if done || shown == Some(at) {
        Due::Nothing
    } else if at > now {
        Due::Later(at)
    } else if now - at < LATE_MS {
        Due::Show
    } else {
        Due::Skip
    }
}

/// What a look at the notes found.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Notes to show now.
    pub show: Vec<String>,
    /// The next reminder's time, if any.
    pub next: Option<u64>,
    /// What's been shown (id -> time) after this look: the shown and the
    /// skipped added; notes gone, Done or given a new time dropped.
    pub shown: BTreeMap<String, u64>,
}

/// Look at every note at `now`, given what was shown before.
pub fn plan(notes: &[Note], shown: &BTreeMap<String, u64>, now: u64) -> Plan {
    let mut out = Plan::default();
    for n in notes {
        let before = shown.get(&n.id).copied();
        match due(n.remind_at, n.remind_done, before, now) {
            Due::Nothing => {
                // Still shown for this very time: keep it.
                if let (Some(at), Some(b)) = (n.remind_at, before) {
                    if at == b && !n.remind_done {
                        out.shown.insert(n.id.clone(), at);
                    }
                }
            }
            Due::Later(at) => out.next = Some(out.next.map_or(at, |x| x.min(at))),
            Due::Show => {
                out.show.push(n.id.clone());
                out.shown.insert(n.id.clone(), n.remind_at.unwrap_or(0));
            }
            Due::Skip => {
                out.shown.insert(n.id.clone(), n.remind_at.unwrap_or(0));
            }
        }
    }
    out
}

/// How long to sleep after a look at `now`: until the next reminder, at
/// most `MAX_SLEEP_MS`.
pub fn sleep_for(next: Option<u64>, now: u64) -> Duration {
    let ms = next.map_or(MAX_SLEEP_MS, |at| at.saturating_sub(now).min(MAX_SLEEP_MS));
    Duration::from_millis(ms.max(1))
}

/// The notification: the title ("Reminder" when the note has none), and the
/// body's first line that isn't blank (checklist boxes left out).
pub fn text(n: &Note) -> (String, String) {
    let title = if n.title.trim().is_empty() {
        "Reminder".to_string()
    } else {
        n.title.trim().to_string()
    };
    let line = n
        .body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let line = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find_map(|b| line.strip_prefix(b))
        .unwrap_or(line);
    (title, line.chars().take(MAX_LINE).collect())
}

/// The reminders shown, where that's kept, and the thread's wake-up call.
pub struct NoteReminders {
    kv: Option<FileKv>,
    shown: Mutex<BTreeMap<String, u64>>,
    /// Set by `changed`; the thread waits on it.
    poke: (Mutex<bool>, Condvar),
}

impl NoteReminders {
    pub fn load(app: &AppHandle) -> Self {
        let kv = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| FileKv::new(dir.join("note_reminders.json")));
        let shown = kv
            .as_ref()
            .and_then(|kv| kv.get(KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        NoteReminders {
            kv,
            shown: Mutex::new(shown),
            poke: (Mutex::new(false), Condvar::new()),
        }
    }

    /// Look at the notes now: what to show, and when to look again. What's
    /// shown is saved before it's shown, so it can't show twice.
    fn look(&self, notes: &[Note], now: u64) -> (Vec<String>, Option<u64>) {
        let Ok(mut shown) = self.shown.lock() else {
            return (Vec::new(), None);
        };
        let p = plan(notes, &shown, now);
        if p.shown != *shown {
            *shown = p.shown;
            if let (Some(kv), Ok(json)) = (&self.kv, serde_json::to_string(&*shown)) {
                if let Err(e) = kv.set(KEY, &json) {
                    eprintln!("[reminders] couldn't save: {e}");
                }
            }
        }
        (p.show, p.next)
    }

    fn wait(&self, d: Duration) {
        let (lock, cv) = &self.poke;
        let Ok(mut poked) = lock.lock() else {
            return;
        };
        if !*poked {
            poked = match cv.wait_timeout(poked, d) {
                Ok((g, _)) => g,
                Err(_) => return,
            };
        }
        *poked = false;
    }
}

/// The notes changed (saved here, or from another device): look again now.
pub fn changed(app: &AppHandle) {
    if let Some(state) = app.try_state::<NoteReminders>() {
        let (lock, cv) = &state.poke;
        if let Ok(mut poked) = lock.lock() {
            *poked = true;
            cv.notify_all();
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Show reminders as they come due (see the top of this file).
pub fn spawn(app: AppHandle) {
    std::thread::Builder::new()
        .name("note-reminders".into())
        .spawn(move || loop {
            let (Some(state), Some(engine)) =
                (app.try_state::<NoteReminders>(), app.try_state::<Eng>())
            else {
                std::thread::sleep(Duration::from_secs(5));
                continue;
            };
            let notes = engine.notes();
            let now = now_ms();
            let (show, next) = state.look(&notes, now);
            for id in show {
                if let Some(n) = notes.iter().find(|n| n.id == id) {
                    let (title, body) = text(n);
                    crate::notify::post(&app, &title, &body);
                    let _ = app.emit("note-reminder", NoteView::from(n.clone()));
                }
            }
            state.wait(sleep_for(next, now));
        })
        .ok();
}

/// Save note `id` with this reminder (the rest as it is).
fn set_reminder(
    app: &AppHandle,
    engine: &Eng,
    id: &str,
    at: Option<u64>,
    done: bool,
) -> Result<NoteView, CoreError> {
    let n = engine
        .note(id)
        .ok_or_else(|| CoreError::InvalidInput("no such note".into()))?;
    let saved = engine.save_note(
        Some(id),
        &n.title,
        &n.body,
        n.pinned,
        n.checklist,
        n.color.as_deref(),
        at,
        done,
    )?;
    changed(app);
    Ok(saved.into())
}

/// Mark a note's reminder Done (on every device, as they sync).
#[tauri::command]
#[specta::specta]
pub fn note_remind_done(
    id: String,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<NoteView, CoreError> {
    let at = engine.note(&id).and_then(|n| n.remind_at);
    set_reminder(&app, &engine, &id, at, true)
}

/// Remind at `at` (unix ms: snooze, or a new time), or no more (null).
#[tauri::command]
#[specta::specta]
pub fn note_remind_at(
    id: String,
    at: Option<f64>,
    app: AppHandle,
    engine: State<Eng>,
) -> Result<NoteView, CoreError> {
    let at = at.filter(|t| t.is_finite() && *t > 0.0).map(|t| t as u64);
    set_reminder(&app, &engine, &id, at, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u64 = 3600 * 1000;

    fn note(id: &str, at: Option<u64>, done: bool) -> Note {
        Note {
            id: id.into(),
            title: id.into(),
            remind_at: at,
            remind_done: done,
            ..Note::default()
        }
    }

    #[test]
    fn due_by_the_rules() {
        let now = 100 * H;
        assert_eq!(due(None, false, None, now), Due::Nothing);
        assert_eq!(due(Some(now + 5), false, None, now), Due::Later(now + 5));
        assert_eq!(due(Some(now), false, None, now), Due::Show);
        assert_eq!(due(Some(now - 23 * H), false, None, now), Due::Show);
        assert_eq!(due(Some(now - 24 * H), false, None, now), Due::Skip);
        // Done, or shown for this time: nothing.
        assert_eq!(due(Some(now - H), true, None, now), Due::Nothing);
        assert_eq!(due(Some(now - H), false, Some(now - H), now), Due::Nothing);
        // Shown for an older time (snoozed since): a new reminder.
        assert_eq!(due(Some(now - H), false, Some(now - 2 * H), now), Due::Show);
    }

    #[test]
    fn the_next_time_is_the_soonest_pending() {
        let now = 100 * H;
        let notes = [
            note("a", Some(now + 3 * H), false),
            note("b", Some(now + H), false),
            note("c", Some(now + 30), true),
            note("d", None, false),
        ];
        let p = plan(&notes, &BTreeMap::new(), now);
        assert!(p.show.is_empty());
        assert_eq!(p.next, Some(now + H));
        assert_eq!(sleep_for(p.next, now), Duration::from_millis(MAX_SLEEP_MS));
        assert_eq!(sleep_for(Some(now + 500), now), Duration::from_millis(500));
        assert_eq!(sleep_for(None, now), Duration::from_millis(MAX_SLEEP_MS));
        // One already due wakes at once.
        assert_eq!(sleep_for(Some(now - 5), now), Duration::from_millis(1));
    }

    #[test]
    fn missed_on_start_shows_once_if_under_a_day_late() {
        // The app starts: one came due an hour ago, one two days ago.
        let now = 100 * H;
        let notes = [
            note("recent", Some(now - H), false),
            note("old", Some(now - 48 * H), false),
        ];
        let p = plan(&notes, &BTreeMap::new(), now);
        assert_eq!(p.show, ["recent"]);
        assert_eq!(p.shown.get("old"), Some(&(now - 48 * H)), "skipped");
        // Started again a minute later: neither shows again.
        let again = plan(&notes, &p.shown, now + 60_000);
        assert!(again.show.is_empty());
        assert_eq!(again.shown, p.shown);
    }

    #[test]
    fn snooze_done_and_delete_change_what_is_kept() {
        let now = 100 * H;
        let shown: BTreeMap<String, u64> = [
            ("s".into(), now - H),
            ("d".into(), now - H),
            ("gone".into(), now - H),
        ]
        .into();
        let notes = [
            note("s", Some(now + 10 * 60_000), false), // snoozed 10 min
            note("d", Some(now - H), true),            // Done
        ];
        let p = plan(&notes, &shown, now);
        assert!(p.show.is_empty());
        assert_eq!(p.next, Some(now + 10 * 60_000));
        assert!(p.shown.is_empty(), "{:?}", p.shown);
        // The snoozed one shows at its new time.
        let later = plan(&notes, &p.shown, now + 10 * 60_000);
        assert_eq!(later.show, ["s"]);
    }

    #[test]
    fn the_notification_says_the_title_and_first_line() {
        let n = Note {
            title: "  Groceries ".into(),
            body: "\n  \n[ ] milk\n[x] eggs".into(),
            ..Note::default()
        };
        assert_eq!(text(&n), ("Groceries".into(), "milk".into()));
        let untitled = Note {
            body: "x".repeat(500),
            ..Note::default()
        };
        let (t, b) = text(&untitled);
        assert_eq!(t, "Reminder");
        assert_eq!(b.chars().count(), MAX_LINE);
    }
}
