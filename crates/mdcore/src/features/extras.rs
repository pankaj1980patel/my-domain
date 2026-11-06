//! State for the gallery, photo backup, battery alerts, remote input, Do
//! Not Disturb, screen sharing, the phone's audio, synced folders, shared
//! device labels, notes, the connection check, the phone's camera and timers, in one place so the
//! engine's context gains one field for all of them.

#[derive(Default)]
pub struct Extras {
    pub gallery: super::gallery::Gallery,
    pub backup: super::backup::Backup,
    pub battery: super::battery::BatteryAlerts,
    pub input: super::input::Input,
    pub dnd: super::dnd::Dnd,
    pub screen: super::screen::Screen,
    pub audio: super::audio_share::AudioShare,
    pub sync: super::folder_sync::FolderSync,
    pub labels: super::labels::Labels,
    pub notes: super::notes::Notes,
    pub diag: super::diag::Diag,
    pub camera: super::camera::Camera,
    pub timers: super::timers::Timers,
}
