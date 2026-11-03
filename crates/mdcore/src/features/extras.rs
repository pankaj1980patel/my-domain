//! State for the gallery, photo backup, battery alerts, remote input, Do
//! Not Disturb and screen sharing, in one place so the engine's context gains one field for
//! all of them.

#[derive(Default)]
pub struct Extras {
    pub gallery: super::gallery::Gallery,
    pub backup: super::backup::Backup,
    pub battery: super::battery::BatteryAlerts,
    pub input: super::input::Input,
    pub dnd: super::dnd::Dnd,
    pub screen: super::screen::Screen,
}
