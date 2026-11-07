//! The GlobalShortcuts portal: keyboard shortcuts that work in any app on
//! Wayland. The compositor may show the list for the user to confirm or
//! change the keys; a press comes back as an `Activated` signal.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use zbus::blocking::MessageIterator;
use zbus::message::Type;
use zbus::MatchRule;
use zvariant::{OwnedObjectPath, Value};

use super::{Opts, Portal};
use crate::caps::{DEST, GLOBAL_SHORTCUTS, PATH};
use crate::response;

/// One shortcut to bind.
#[derive(Clone, Debug)]
pub struct Wanted {
    pub id: String,
    pub description: String,
    /// The keys asked for (`keys::trigger`); the user may pick others.
    pub trigger: Option<String>,
}

/// Shortcuts bound for as long as this lives.
pub struct Shortcuts {
    _p: Portal,
    stopped: Arc<AtomicBool>,
    /// The ids the compositor took.
    pub bound: Vec<String>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("the shortcuts portal didn\u{2019}t answer: {e}")
}

impl Shortcuts {
    /// Bind `wanted`; `on_press` gets a shortcut's id each time it's
    /// pressed (on the portal's thread).
    pub fn bind(
        wanted: &[Wanted],
        on_press: impl Fn(String) + Send + 'static,
    ) -> Result<Shortcuts, String> {
        let p = Portal::new()?;
        let session = p.create_session(GLOBAL_SHORTCUTS, Opts::new())?;
        // Listen before binding, so no press is missed.
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(GLOBAL_SHORTCUTS)
            .and_then(|b| b.member("Activated"))
            .map_err(err)?
            .build();
        let it = MessageIterator::for_match_rule(rule, &p.conn, Some(16)).map_err(err)?;
        let list: Vec<(String, HashMap<&str, Value<'_>>)> = wanted
            .iter()
            .map(|w| {
                let mut m = HashMap::new();
                m.insert("description", Value::from(w.description.as_str()));
                if let Some(t) = &w.trigger {
                    m.insert("preferred_trigger", Value::from(t.as_str()));
                }
                (w.id.clone(), m)
            })
            .collect();
        let (opts, answer) = p.ask("bind", Opts::new())?;
        p.conn
            .call_method(
                Some(DEST),
                PATH,
                Some(GLOBAL_SHORTCUTS),
                "BindShortcuts",
                &(&session, list, "", opts),
            )
            .map_err(err)?;
        let bound = response::shortcut_ids(&p.wait(answer)?);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let mine = session.clone();
        std::thread::Builder::new()
            .name("global-shortcuts".into())
            .spawn(move || {
                for m in it {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(m) = m else { break };
                    let Ok((s, id, _time, _opts)) =
                        m.body()
                            .deserialize::<(OwnedObjectPath, String, u64, response::Dict)>()
                    else {
                        continue;
                    };
                    if s == mine {
                        on_press(id);
                    }
                }
            })
            .map_err(err)?;
        Ok(Shortcuts {
            _p: p,
            stopped,
            bound,
        })
    }
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}
