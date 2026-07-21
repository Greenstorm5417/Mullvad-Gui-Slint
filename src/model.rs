#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoCoordinate {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TunnelStatus {
    Unavailable(String),
    Disconnected {
        location: Option<String>,
        coordinates: Option<GeoCoordinate>,
    },
    Connecting {
        location: Option<String>,
        coordinates: Option<GeoCoordinate>,
    },
    Connected {
        location: Option<String>,
        coordinates: Option<GeoCoordinate>,
    },
    Disconnecting,
    Error(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BooleanSetting {
    AllowLan,
    AutoConnect,
    EnableIpv6,
    LockdownMode,
    ShowBetaReleases,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppSettings {
    pub allow_lan: bool,
    pub auto_connect: bool,
    pub enable_ipv6: bool,
    pub lockdown_mode: bool,
    pub show_beta_releases: bool,
}

impl AppSettings {
    pub fn value(&self, setting: BooleanSetting) -> bool {
        match setting {
            BooleanSetting::AllowLan => self.allow_lan,
            BooleanSetting::AutoConnect => self.auto_connect,
            BooleanSetting::EnableIpv6 => self.enable_ipv6,
            BooleanSetting::LockdownMode => self.lockdown_mode,
            BooleanSetting::ShowBetaReleases => self.show_beta_releases,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountStatus {
    LoggedOut,
    LoggedIn {
        account_number: String,
        device_name: String,
        expiry: Option<String>,
    },
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceSummary {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayLocation {
    pub label: String,
    pub country_code: String,
    pub city_code: Option<String>,
    pub hostname: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SplitTunnelState {
    pub process_ids: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsBlocker {
    Ads,
    AdultContent,
    Gambling,
    Malware,
    SocialMedia,
    Trackers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MultihopMode {
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObfuscationMode {
    Auto,
    Off,
    WireguardPort,
    UdpOverTcp,
    Shadowsocks,
    Quic,
    Lwo,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvancedSettings {
    pub block_ads: bool,
    pub block_adult_content: bool,
    pub block_gambling: bool,
    pub block_malware: bool,
    pub block_social_media: bool,
    pub block_trackers: bool,
    pub daita: bool,
    pub mtu: Option<u32>,
    pub multihop: MultihopMode,
    pub obfuscation: ObfuscationMode,
    pub quantum_resistant: bool,
}

impl AdvancedSettings {
    pub fn blocker_enabled(&self, blocker: DnsBlocker) -> bool {
        match blocker {
            DnsBlocker::Ads => self.block_ads,
            DnsBlocker::AdultContent => self.block_adult_content,
            DnsBlocker::Gambling => self.block_gambling,
            DnsBlocker::Malware => self.block_malware,
            DnsBlocker::SocialMedia => self.block_social_media,
            DnsBlocker::Trackers => self.block_trackers,
        }
    }
}

impl TunnelStatus {
    pub fn headline(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "DISCONNECTED FROM SYSTEM SERVICE",
            Self::Disconnected { .. } => "DISCONNECTED",
            Self::Connecting { .. } => "CONNECTING...",
            Self::Connected { .. } => "CONNECTED",
            Self::Disconnecting => "DISCONNECTING...",
            Self::Error(_) => "FAILED TO SECURE CONNECTION",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::Unavailable(message) | Self::Error(message) => message.clone(),
            Self::Disconnected { location, .. }
            | Self::Connecting { location, .. }
            | Self::Connected { location, .. } => location
                .clone()
                .unwrap_or_else(|| "Location unavailable".to_owned()),
            Self::Disconnecting => "Closing the VPN tunnel".to_owned(),
        }
    }

    pub fn action_label(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "Disconnect",
            Self::Connecting { .. } => "Cancel",
            Self::Unavailable(_)
            | Self::Disconnected { .. }
            | Self::Disconnecting
            | Self::Error(_) => "Connect",
        }
    }

    pub fn wants_disconnect(&self) -> bool {
        matches!(self, Self::Connected { .. } | Self::Connecting { .. })
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Connecting { .. } | Self::Disconnecting)
    }

    pub fn style_class(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "connected",
            Self::Connecting { .. } | Self::Disconnecting => "transitioning",
            Self::Disconnected { .. } | Self::Unavailable(_) | Self::Error(_) => "disconnected",
        }
    }

    pub fn coordinates(&self) -> Option<GeoCoordinate> {
        match self {
            Self::Disconnected { coordinates, .. }
            | Self::Connecting { coordinates, .. }
            | Self::Connected { coordinates, .. } => *coordinates,
            Self::Unavailable(_) | Self::Disconnecting | Self::Error(_) => None,
        }
    }
}
