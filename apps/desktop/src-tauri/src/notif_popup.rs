// The pop-up for a phone notification, answerable where the OS lets it be:
//
//  - macOS (`mac-notification-sys`, the library the notification plugin
//    already uses): a reply field when the notification takes a reply,
//    else its buttons and "Mute <app>" under Options; Dismiss dismisses it
//    on the phone too; a click opens the phone's page here. Each such
//    pop-up waits on its own thread until it's answered or gone, so at most
//    `MAX_WAITING` wait at once; past that they're plain.
//  - Windows (a toast built here, WinRT): a reply box and its buttons
//    (at most five in all, Mute among them), the same answers; one closed
//    with its X is dismissed on the phone; one gone on the phone is taken
//    off the screen.
//  - Linux, and anything without a key: the plugin's plain pop-up.
//
// Only while the window isn't focused (the device page shows the same) and
// as any everyday pop-up: off with "Show notifications", held during the
// phone's Do Not Disturb when the computer follows it.

// Linux pops up plain ones only: the answering parts are for the others
// (and the tests).
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

use std::path::PathBuf;

use base64::Engine as _;
use tauri::{AppHandle, Emitter, Manager};

use crate::notify::{self, Kind};
use crate::phone_notifs::{self, NotifButton, PhoneNotif, HIDDEN};
use crate::Eng;

/// Pop-ups waiting for an answer at once (macOS).
#[cfg(target_os = "macos")]
const MAX_WAITING: usize = 12;
/// The most buttons a Windows toast takes.
#[cfg_attr(not(windows), allow(dead_code))]
const MAX_TOAST_BUTTONS: usize = 5;
/// Pictures kept for pop-ups, round robin.
const IMAGE_SLOTS: u32 = 24;

/// What a pop-up shows and can answer.
#[derive(Clone, Debug)]
pub struct PopUp {
    pub from: String,
    pub key: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub buttons: Vec<NotifButton>,
    pub reply: Option<NotifButton>,
    /// A picture for it (the notification's, else the sender's), on disk.
    pub image: Option<PathBuf>,
    /// The app's icon, on disk.
    pub icon: Option<PathBuf>,
}

/// How it was answered.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Reply(String),
    /// The phone's index of the button pressed.
    Button(u32),
    Dismiss,
    Mute,
    Open,
    Nothing,
}

/// The title and text a pop-up shows: a private one only says it came.
pub fn texts(n: &PhoneNotif, device: &str) -> (String, String) {
    let app = if n.app.is_empty() { device } else { &n.app };
    if n.private {
        return (app.to_string(), HIDDEN.to_string());
    }
    let title = n
        .conv_title
        .clone()
        .filter(|t| !t.is_empty())
        .or_else(|| Some(n.title.clone()).filter(|t| !t.trim().is_empty()))
        .unwrap_or_else(|| app.to_string());
    let body = match (&n.sender, n.group_chat) {
        (Some(s), true) if !n.body.starts_with(s.as_str()) => format!("{s}: {}", n.body),
        _ => n.body.clone(),
    };
    (title, body)
}

/// The macOS Options menu: the buttons (commas would split them), then
/// "Mute <app>".
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn menu(p: &PopUp) -> Vec<String> {
    let mut m: Vec<String> = p
        .buttons
        .iter()
        .map(|b| b.title.replace(',', " "))
        .collect();
    if !p.app_name.is_empty() {
        m.push(mute_label(&p.app_name));
    }
    m
}

fn mute_label(app: &str) -> String {
    format!("Mute {}", app.replace(',', " "))
}

/// The answer for menu item `title` (macOS gives back its title).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn answer_for_menu(p: &PopUp, title: &str) -> Answer {
    if !p.app_name.is_empty() && title == mute_label(&p.app_name) {
        return Answer::Mute;
    }
    p.buttons
        .iter()
        .find(|b| b.title.replace(',', " ") == title)
        .map(|b| Answer::Button(b.index))
        .unwrap_or(Answer::Nothing)
}

/// The answer for a toast's `arguments` (and the typed `text`), only for
/// a button it really has.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn answer_for_args(p: &PopUp, arguments: &str, text: Option<&str>) -> Answer {
    match arguments {
        "reply" => match (text.map(str::trim), &p.reply) {
            (Some(t), Some(_)) if !t.is_empty() => Answer::Reply(t.to_string()),
            _ => Answer::Nothing,
        },
        "dismiss" => Answer::Dismiss,
        "mute" => Answer::Mute,
        "" | "open" => Answer::Open,
        other => other
            .strip_prefix("a:")
            .and_then(|i| i.parse::<u32>().ok())
            .filter(|i| p.buttons.iter().any(|b| b.index == *i))
            .map(Answer::Button)
            .unwrap_or(Answer::Nothing),
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg_attr(not(windows), allow(dead_code))]
fn file_uri(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    format!("file:///{}", s.trim_start_matches('/'))
}

/// A Windows toast for `p`: its texts, the app icon and picture, a reply
/// box, its buttons, Mute and Dismiss (five at most: the reply's Send and
/// Dismiss first, then Mute, then the notification's own).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn toast_xml(p: &PopUp) -> String {
    let mut x = String::from(r#"<toast launch="open"><visual><binding template="ToastGeneric">"#);
    x.push_str(&format!("<text>{}</text>", xml_escape(&p.title)));
    if !p.body.is_empty() {
        x.push_str(&format!("<text>{}</text>", xml_escape(&p.body)));
    }
    if !p.app_name.is_empty() {
        x.push_str(&format!(
            r#"<text placement="attribution">{}</text>"#,
            xml_escape(&p.app_name)
        ));
    }
    if let Some(icon) = &p.icon {
        x.push_str(&format!(
            r#"<image placement="appLogoOverride" src="{}"/>"#,
            xml_escape(&file_uri(icon))
        ));
    }
    if let Some(img) = &p.image {
        x.push_str(&format!(r#"<image src="{}"/>"#, xml_escape(&file_uri(img))));
    }
    x.push_str("</binding></visual><actions>");
    let mut room = MAX_TOAST_BUTTONS;
    if let Some(r) = &p.reply {
        let hint = if r.title.is_empty() {
            "Reply"
        } else {
            &r.title
        };
        x.push_str(&format!(
            r#"<input id="reply" type="text" placeHolderContent="{}"/>"#,
            xml_escape(hint)
        ));
        x.push_str(r#"<action content="Send" arguments="reply" hint-inputId="reply"/>"#);
        room -= 1;
    }
    room -= 1; // Dismiss, last.
    let mute = !p.app_name.is_empty();
    if mute {
        room -= 1;
    }
    for b in p.buttons.iter().take(room) {
        x.push_str(&format!(
            r#"<action content="{}" arguments="a:{}"/>"#,
            xml_escape(&b.title),
            b.index
        ));
    }
    if mute {
        x.push_str(&format!(
            r#"<action content="{}" arguments="mute"/>"#,
            xml_escape(&mute_label(&p.app_name))
        ));
    }
    x.push_str(r#"<action content="Dismiss" arguments="dismiss"/>"#);
    x.push_str("</actions></toast>");
    x
}

/// Pop up notification `n` from `from` (the app's icon at `icon`), if a
/// pop-up may show now.
pub fn show(app: &AppHandle, n: &PhoneNotif, icon: Option<PathBuf>) {
    if !notify::away(app) {
        return;
    }
    let device = notify::device_name(app, &n.from);
    let (title, body) = texts(n, &device);
    if !notify::may_show(app, Kind::Everyday, &title) {
        return;
    }
    if let Some(key) = &n.key {
        let p = PopUp {
            from: n.from.clone(),
            key: key.clone(),
            app_name: n.app.clone(),
            title: title.clone(),
            body: body.clone(),
            buttons: n.buttons.clone(),
            reply: n.reply.clone(),
            image: image_file(app, n),
            icon,
        };
        if interactive(app, p) {
            return;
        }
    }
    notify::show_plain(app, &title, &body);
}

/// The notification's picture (else the sender's) written where the OS can
/// read it.
fn image_file(app: &AppHandle, n: &PhoneNotif) -> Option<PathBuf> {
    let url = n.picture.as_ref().or(n.avatar.as_ref())?;
    let b64 = url.strip_prefix("data:image/jpeg;base64,")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let dir = app.path().app_cache_dir().ok()?.join("notif-images");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{}.jpg", n.id % IMAGE_SLOTS));
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// Do what the pop-up was answered with.
fn act(app: &AppHandle, p: &PopUp, a: Answer) {
    let engine = app.try_state::<Eng>();
    let send = |index: i64, text: Option<&str>| match &engine {
        Some(e) => e.notification_action(&p.from, &p.key, index, text).is_ok(),
        None => false,
    };
    match a {
        Answer::Reply(text) => {
            if let Some(r) = &p.reply {
                if send(i64::from(r.index), Some(&text)) {
                    phone_notifs::on_acted(app, &p.from, &p.key, r.index as i32);
                }
            }
        }
        Answer::Button(i) => {
            if send(i64::from(i), None) {
                phone_notifs::on_acted(app, &p.from, &p.key, i as i32);
            }
        }
        Answer::Dismiss => {
            send(-1, None);
            crate::notif_history::on_dismissed(app, &p.from, &p.key);
            phone_notifs::on_removed(app, &p.from, &p.key);
        }
        Answer::Mute => {
            let muted = crate::notif_history::mute(app, &p.app_name);
            phone_notifs::on_muted(app, &muted);
            let _ = app.emit("muted-apps-changed", &muted);
        }
        Answer::Open => {
            crate::tray::show_window(app);
            let _ = app.emit("open-device", serde_json::json!({ "node_id": p.from }));
        }
        Answer::Nothing => {}
    }
}

/// `from`'s notification `key` is gone: off the screen too, where the OS
/// lets a pop-up be taken back (Windows).
pub fn withdraw(from: &str, key: &str) {
    #[cfg(windows)]
    win::hide(&format!("{from}\u{1}{key}"));
    #[cfg(not(windows))]
    let _ = (from, key);
}

#[cfg(target_os = "macos")]
fn interactive(app: &AppHandle, p: PopUp) -> bool {
    use mac_notification_sys::{MainButton, Notification, NotificationResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static WAITING: AtomicUsize = AtomicUsize::new(0);
    if WAITING.fetch_add(1, Ordering::AcqRel) >= MAX_WAITING {
        WAITING.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    let app = app.clone();
    let bundle = if tauri::is_dev() {
        "com.apple.Terminal".to_string()
    } else {
        app.config().identifier.clone()
    };
    std::thread::spawn(move || {
        // Once for the process (the plugin does the same); an error says
        // it was set already.
        let _ = mac_notification_sys::set_application(&bundle);
        let menu = menu(&p);
        let items: Vec<&str> = menu.iter().map(String::as_str).collect();
        let image = p.image.as_ref().map(|i| i.to_string_lossy().to_string());
        let icon = p.icon.as_ref().map(|i| i.to_string_lossy().to_string());
        let mut n = Notification::new();
        n.title(&p.title).message(&p.body).close_button("Dismiss");
        if !p.app_name.is_empty() {
            n.subtitle(&p.app_name);
        }
        let reply_hint = p.reply.as_ref().map(|r| {
            if r.title.is_empty() {
                "Reply".to_string()
            } else {
                r.title.clone()
            }
        });
        if let Some(hint) = &reply_hint {
            n.main_button(MainButton::Response(hint));
        } else if items.len() == 1 {
            n.main_button(MainButton::SingleAction(items[0]));
        } else if !items.is_empty() {
            n.main_button(MainButton::DropdownActions("Options", &items));
        }
        if let Some(img) = &image {
            n.content_image(img);
        }
        if let Some(icon) = &icon {
            n.app_icon(icon);
        }
        let answer = match n.send() {
            Ok(NotificationResponse::Reply(text)) if !text.trim().is_empty() => {
                Answer::Reply(text.trim().to_string())
            }
            Ok(NotificationResponse::ActionButton(title)) => answer_for_menu(&p, &title),
            Ok(NotificationResponse::CloseButton(_)) => Answer::Dismiss,
            Ok(NotificationResponse::Click) => Answer::Open,
            Ok(_) => Answer::Nothing,
            Err(e) => {
                eprintln!("[notify] couldn't show a notification: {e}");
                Answer::Nothing
            }
        };
        WAITING.fetch_sub(1, Ordering::AcqRel);
        act(&app, &p, answer);
    });
    true
}

#[cfg(windows)]
fn interactive(app: &AppHandle, p: PopUp) -> bool {
    // The installed app's own id; a development build borrows
    // PowerShell's, as the notification plugin does.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_string_lossy().to_string()))
        .unwrap_or_default();
    let dev = exe_dir.ends_with("target\\debug") || exe_dir.ends_with("target\\release");
    let aumid = if dev {
        r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe".to_string()
    } else {
        app.config().identifier.clone()
    };
    let tag = format!("{}\u{1}{}", p.from, p.key);
    let xml = toast_xml(&p);
    let app = app.clone();
    let shown = win::show(
        &aumid,
        &xml,
        tag,
        move |arguments: String, text: Option<String>| {
            let answer = answer_for_args(&p, &arguments, text.as_deref());
            let (app, p) = (app.clone(), p.clone());
            std::thread::spawn(move || act(&app, &p, answer));
        },
    );
    match shown {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[notify] couldn't show a toast: {e}");
            false
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn interactive(_app: &AppHandle, _p: PopUp) -> bool {
    false
}

#[cfg(windows)]
mod win {
    use std::sync::{Arc, Mutex, OnceLock};

    use windows::core::{Interface, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::{IPropertyValue, TypedEventHandler};
    use windows::UI::Notifications::{
        ToastActivatedEventArgs, ToastDismissalReason, ToastNotification, ToastNotificationManager,
        ToastNotifier,
    };

    /// Toasts on screen, by tag, oldest first.
    type Shown = Vec<(String, ToastNotifier, ToastNotification)>;
    const MAX_SHOWN: usize = 64;

    fn shown() -> &'static Mutex<Shown> {
        static S: OnceLock<Mutex<Shown>> = OnceLock::new();
        S.get_or_init(|| Mutex::new(Vec::new()))
    }

    fn forget(tag: &str) -> Option<(ToastNotifier, ToastNotification)> {
        let mut s = shown().lock().ok()?;
        let i = s.iter().position(|(t, _, _)| t == tag)?;
        let (_, n, t) = s.remove(i);
        Some((n, t))
    }

    /// Take toast `tag` off the screen.
    pub fn hide(tag: &str) {
        if let Some((notifier, toast)) = forget(tag) {
            let _ = notifier.Hide(&toast);
        }
    }

    /// Show toast `xml` as app `aumid`, under `tag`; `on(arguments, text)`
    /// when it's answered ("dismiss" when closed with its X).
    pub fn show(
        aumid: &str,
        xml: &str,
        tag: String,
        on: impl Fn(String, Option<String>) + Send + Sync + 'static,
    ) -> windows::core::Result<()> {
        hide(&tag);
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        let on = Arc::new(on);
        let (on_a, tag_a) = (on.clone(), tag.clone());
        toast.Activated(&TypedEventHandler::new(
            move |_, insp: windows::core::Ref<windows::core::IInspectable>| {
                let Ok(insp) = insp.ok() else {
                    return Ok(());
                };
                let args: ToastActivatedEventArgs = insp.cast()?;
                let arguments = args.Arguments()?.to_string();
                let text = args
                    .UserInput()
                    .ok()
                    .and_then(|v| v.Lookup(&HSTRING::from("reply")).ok())
                    .and_then(|v| v.cast::<IPropertyValue>().ok())
                    .and_then(|v| v.GetString().ok())
                    .map(|h| h.to_string());
                forget(&tag_a);
                on_a(arguments, text);
                Ok(())
            },
        ))?;
        let (on_d, tag_d) = (on, tag.clone());
        toast.Dismissed(
            &TypedEventHandler::new(
                move |_,
                      args: windows::core::Ref<
                    windows::UI::Notifications::ToastDismissedEventArgs,
                >| {
                    let Ok(args) = args.ok() else {
                        return Ok(());
                    };
                    if args.Reason()? == ToastDismissalReason::UserCanceled {
                        forget(&tag_d);
                        on_d("dismiss".to_string(), None);
                    }
                    Ok(())
                },
            ),
        )?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid))?;
        notifier.Show(&toast)?;
        if let Ok(mut s) = shown().lock() {
            s.push((tag, notifier, toast));
            if s.len() > MAX_SHOWN {
                s.remove(0);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn popup() -> PopUp {
        PopUp {
            from: "p".into(),
            key: "k".into(),
            app_name: "Chat".into(),
            title: "Asha & Ravi <3".into(),
            body: "Dinner at 8?".into(),
            buttons: vec![
                NotifButton {
                    index: 1,
                    title: "Mark as read".into(),
                },
                NotifButton {
                    index: 3,
                    title: "Like, love".into(),
                },
            ],
            reply: Some(NotifButton {
                index: 0,
                title: "Reply".into(),
            }),
            image: None,
            icon: Some(PathBuf::from("/tmp/icons/com.chat.jpg")),
        }
    }

    #[test]
    fn answers_map_back_to_the_phones_buttons() {
        let p = popup();
        assert_eq!(answer_for_args(&p, "a:1", None), Answer::Button(1));
        assert_eq!(answer_for_args(&p, "a:3", None), Answer::Button(3));
        // A button it doesn't have, or junk: nothing.
        assert_eq!(answer_for_args(&p, "a:2", None), Answer::Nothing);
        assert_eq!(answer_for_args(&p, "a:x", None), Answer::Nothing);
        assert_eq!(
            answer_for_args(&p, "reply", Some(" On my way ")),
            Answer::Reply("On my way".into())
        );
        assert_eq!(answer_for_args(&p, "reply", Some("  ")), Answer::Nothing);
        assert_eq!(answer_for_args(&p, "dismiss", None), Answer::Dismiss);
        assert_eq!(answer_for_args(&p, "mute", None), Answer::Mute);
        assert_eq!(answer_for_args(&p, "open", None), Answer::Open);
        let no_reply = PopUp {
            reply: None,
            ..popup()
        };
        assert_eq!(
            answer_for_args(&no_reply, "reply", Some("x")),
            Answer::Nothing
        );

        assert_eq!(menu(&p), ["Mark as read", "Like  love", "Mute Chat"]);
        assert_eq!(answer_for_menu(&p, "Like  love"), Answer::Button(3));
        assert_eq!(answer_for_menu(&p, "Mute Chat"), Answer::Mute);
        assert_eq!(answer_for_menu(&p, "Other"), Answer::Nothing);
    }

    #[test]
    fn a_toast_has_a_reply_box_buttons_mute_and_dismiss_within_five() {
        let mut p = popup();
        for i in 4..9 {
            p.buttons.push(NotifButton {
                index: i,
                title: format!("B{i}"),
            });
        }
        let x = toast_xml(&p);
        assert!(x.contains("<text>Asha &amp; Ravi &lt;3</text>"));
        assert!(x.contains(r#"<input id="reply" type="text" placeHolderContent="Reply"/>"#));
        assert!(x.contains(r#"src="file:///tmp/icons/com.chat.jpg""#));
        assert_eq!(x.matches("<action ").count(), MAX_TOAST_BUTTONS);
        assert!(x.contains(r#"arguments="a:1""#) && x.contains(r#"arguments="a:3""#));
        assert!(!x.contains(r#"arguments="a:4""#));
        assert!(x.contains(r#"arguments="mute""#) && x.contains(r#"arguments="dismiss""#));
        // Nothing to answer but dismiss and mute.
        let plain = PopUp {
            reply: None,
            buttons: vec![],
            app_name: String::new(),
            ..popup()
        };
        let x = toast_xml(&plain);
        assert!(!x.contains("<input"));
        assert_eq!(x.matches("<action ").count(), 1);
    }

    #[test]
    fn a_private_one_only_says_it_came() {
        let mut n = crate::phone_notifs::build(
            1,
            "p",
            Some("Bank"),
            "",
            "",
            Some("k"),
            &serde_json::json!([]),
            &Default::default(),
            0.0,
        );
        n.private = true;
        assert_eq!(texts(&n, "Pixel"), ("Bank".to_string(), HIDDEN.to_string()));
        n.app.clear();
        assert_eq!(texts(&n, "Pixel").0, "Pixel");
        // A group chat says who wrote.
        let mut g = crate::phone_notifs::build(
            2,
            "p",
            Some("Chat"),
            "Family",
            "Dinner?",
            Some("k"),
            &serde_json::json!([]),
            &mdcore::features::notif_rich::NotifExtra {
                conv_title: Some("Family".into()),
                group_chat: true,
                sender: Some("Asha".into()),
                ..Default::default()
            },
            0.0,
        );
        assert_eq!(
            texts(&g, "Pixel"),
            ("Family".to_string(), "Asha: Dinner?".to_string())
        );
        g.body = "Asha: Dinner?".into();
        assert_eq!(texts(&g, "Pixel").1, "Asha: Dinner?");
    }
}
