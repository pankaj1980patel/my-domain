//! State for the gallery, photo backup, battery alerts, remote input, Do
//! Not Disturb, screen sharing, the phone's audio, synced folders, shared
//! device labels, notes, the connection check, the phone's camera, timers, the read-later list, the links map, nearby media mode, shared mouse and keyboard, insert from phone, printing, rich notifications, device kinds, app versions, first-run setup, verification codes and wallpapers, in one place so the
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
    pub reconnect: super::reconnect::ReconnectState,
    pub camera: super::camera::Camera,
    pub timers: super::timers::Timers,
    pub read_later: super::read_later::ReadLater,
    pub links_map: super::links_map::LinksMapState,
    pub nearby: super::nearby::NearbyState,
    pub wifi_direct: super::wifi_direct::WifiDirectState,
    pub kvm: super::kvm::Kvm,
    pub insert: super::insert_capture::InsertCapture,
    pub print: super::print::Print,
    pub remote_download: super::remote_download::RemoteDownloads,
    pub notif: super::notif_rich::NotifRich,
    pub device_kind: super::device_kind::DeviceKinds,
    pub app_info: super::app_info::AppInfo,
    pub onboarding: super::onboarding::OnboardingState,
    pub otp: super::otp_relay::OtpRelay,
    pub perms: super::perms::Perms,
    pub wallpaper: super::wallpaper::Wallpapers,
}
