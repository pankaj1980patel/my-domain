//! State for the gallery, photo backup and battery alerts, in one place so
//! the engine's context gains one field for all of them.

#[derive(Default)]
pub struct Extras {
    pub gallery: super::gallery::Gallery,
    pub backup: super::backup::Backup,
    pub battery: super::battery::BatteryAlerts,
}
