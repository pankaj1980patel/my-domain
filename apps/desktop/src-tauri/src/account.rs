//! Settings → Account: change the account password, sign out all devices,
//! the signed-in sessions; and signing in again after the server signed
//! this computer out (`mdcore` emits `signed-out`; the window asks for the
//! password). Each talks to the server, so off the main thread.

use mdcore::registry::AccountSession;
use mdcore::CoreError;
use tauri::{AppHandle, State};

use crate::{off_main, Eng};

/// Change the account password. This computer gets a new session; every
/// other device is signed out by the server.
#[tauri::command]
#[specta::specta]
pub async fn change_account_password(
    app: AppHandle,
    current: String,
    new_password: String,
) -> Result<(), CoreError> {
    off_main(app, move |e| {
        e.change_account_password(&current, &new_password)
    })
    .await
}

/// Sign out every device of the account. With the password this computer
/// signs straight back in (true); without it, it's signed out too.
#[tauri::command]
#[specta::specta]
pub async fn sign_out_all_devices(app: AppHandle, password: String) -> Result<bool, CoreError> {
    off_main(app, move |e| e.sign_out_all_devices(&password)).await
}

/// The account's signed-in sessions, this computer's first.
#[tauri::command]
#[specta::specta]
pub async fn account_sessions(app: AppHandle) -> Result<Vec<AccountSession>, CoreError> {
    off_main(app, |e| e.account_sessions()).await
}

/// Sign out one session from the list.
#[tauri::command]
#[specta::specta]
pub async fn sign_out_session(app: AppHandle, id: String) -> Result<(), CoreError> {
    off_main(app, move |e| e.sign_out_session(&id)).await
}

/// The server signed this computer out and it hasn't signed in again.
#[tauri::command]
#[specta::specta]
pub fn signed_out_elsewhere(engine: State<Eng>) -> bool {
    engine.signed_out_elsewhere()
}

/// Sign in again after that, with the account password only.
#[tauri::command]
#[specta::specta]
pub async fn sign_in_again(app: AppHandle, password: String) -> Result<(), CoreError> {
    off_main(app, move |e| e.sign_in_again(&password)).await
}

/// Shortest account password the server takes.
const MIN_PASSWORD: usize = 6;

/// What stops the change-password form from being sent, as the window's
/// message key; `None` when it can go.
fn password_problem(current: &str, new: &str, repeat: &str) -> Option<&'static str> {
    if current.is_empty() {
        Some("account.needCurrent")
    } else if new.chars().count() < MIN_PASSWORD {
        Some("account.tooShort")
    } else if new.len() > 256 {
        Some("account.tooLong")
    } else if new == current {
        Some("account.same")
    } else if new != repeat {
        Some("account.mismatch")
    } else {
        None
    }
}

/// The change-password form's problem (a message key), if any.
#[tauri::command]
#[specta::specta]
pub fn account_password_problem(
    current: String,
    new_password: String,
    repeat: String,
) -> Option<String> {
    password_problem(&current, &new_password, &repeat).map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_needs_the_current_password_a_long_enough_new_one_typed_twice() {
        assert_eq!(
            password_problem("", "secret1", "secret1"),
            Some("account.needCurrent")
        );
        assert_eq!(
            password_problem("old", "short", "short"),
            Some("account.tooShort")
        );
        // Six characters, counted as characters (not bytes).
        assert_eq!(password_problem("old", "पासवर्डप", "पासवर्डप"), None);
        assert_eq!(
            password_problem("old", &"x".repeat(257), &"x".repeat(257)),
            Some("account.tooLong")
        );
        assert_eq!(
            password_problem("secret1", "secret1", "secret1"),
            Some("account.same")
        );
        assert_eq!(
            password_problem("old", "secret1", "secret2"),
            Some("account.mismatch")
        );
        assert_eq!(password_problem("old", "secret1", "secret1"), None);
    }

    #[test]
    fn every_problem_has_words_in_the_window() {
        let en = include_str!("../../ui/src/i18n/en.ts");
        let hi = include_str!("../../ui/src/i18n/hi.ts");
        for key in [
            "account.needCurrent",
            "account.tooShort",
            "account.tooLong",
            "account.same",
            "account.mismatch",
        ] {
            let quoted = format!("\"{key}\":");
            assert!(en.contains(&quoted), "{key} missing in en.ts");
            assert!(hi.contains(&quoted), "{key} missing in hi.ts");
        }
    }
}
