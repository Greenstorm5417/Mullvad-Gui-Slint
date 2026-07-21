#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoCoordinate {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionDetails {
    pub hostname: Option<String>,
    pub entry_hostname: Option<String>,
    pub in_address: Option<String>,
    pub out_ipv4: Option<String>,
    pub out_ipv6: Option<String>,
    pub protocol: Option<String>,
    pub features: Vec<String>,
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
        details: Option<ConnectionDetails>,
    },
    Connected {
        location: Option<String>,
        coordinates: Option<GeoCoordinate>,
        details: Option<ConnectionDetails>,
    },
    Disconnecting {
        reconnecting: bool,
    },
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
pub struct AccountExpiry {
    pub remaining: String,
    pub paid_until: String,
    pub expired: bool,
    pub show_in_header: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccountStatus {
    LoggedOut,
    LoggedIn {
        account_number: String,
        device_id: String,
        device_name: String,
        expiry: Option<AccountExpiry>,
    },
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceSummary {
    pub id: String,
    pub name: String,
    pub created: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayLocation {
    pub label: String,
    pub country_code: String,
    pub city_code: Option<String>,
    pub hostname: Option<String>,
    pub custom_list_id: Option<String>,
    pub depth: u8,
    pub provider: Option<String>,
    pub owned: Option<bool>,
    pub daita: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayRole {
    Entry,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnershipFilter {
    Any,
    MullvadOwned,
    Rented,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomListSummary {
    pub id: String,
    pub name: String,
    pub locations: Vec<RelayLocation>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LocationSettings {
    pub relays: Vec<RelayLocation>,
    pub recents: Vec<RelayLocation>,
    pub custom_lists: Vec<CustomListSummary>,
    pub providers: Vec<String>,
    pub recents_enabled: bool,
    pub selected_entry: Option<RelayLocation>,
    pub selected_exit: Option<RelayLocation>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SplitTunnelState {
    pub enabled: bool,
    pub applications: Vec<String>,
    pub process_ids: Vec<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpVersionMode {
    Automatic,
    Ipv4,
    Ipv6,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayOverride {
    pub hostname: String,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApiAccessMethodSummary {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub in_use: bool,
    pub custom: bool,
    pub proxy_type: i32,
    pub server: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub cipher: String,
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
    pub custom_dns_enabled: bool,
    pub custom_dns_addresses: Vec<String>,
    pub ip_version: IpVersionMode,
    pub allowed_ips: Vec<String>,
    pub mtu: Option<u32>,
    pub multihop: MultihopMode,
    pub obfuscation: ObfuscationMode,
    pub wireguard_port: Option<u32>,
    pub udp_over_tcp_port: Option<u32>,
    pub shadowsocks_port: Option<u32>,
    pub lwo_port: Option<u32>,
    pub quantum_resistant: bool,
    pub relay_overrides: Vec<RelayOverride>,
    pub userspace_wireguard: bool,
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
            Self::Disconnecting { reconnecting: true } => "RECONNECTING...",
            Self::Disconnecting {
                reconnecting: false,
            } => "DISCONNECTING...",
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
            Self::Disconnecting { reconnecting: true } => "Reconnecting".to_owned(),
            Self::Disconnecting {
                reconnecting: false,
            } => "Closing the VPN tunnel".to_owned(),
        }
    }

    pub fn action_label(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "Disconnect",
            Self::Connecting { .. } => "Cancel",
            Self::Unavailable(_)
            | Self::Disconnected { .. }
            | Self::Disconnecting { .. }
            | Self::Error(_) => "Connect",
        }
    }

    pub fn wants_disconnect(&self) -> bool {
        matches!(self, Self::Connected { .. } | Self::Connecting { .. })
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Connecting { .. } | Self::Disconnecting { .. })
    }

    pub fn style_class(&self) -> &'static str {
        match self {
            Self::Connected { .. } => "connected",
            Self::Connecting { .. } | Self::Disconnecting { .. } => "transitioning",
            Self::Disconnected { .. } | Self::Unavailable(_) | Self::Error(_) => "disconnected",
        }
    }

    pub fn coordinates(&self) -> Option<GeoCoordinate> {
        match self {
            Self::Disconnected { coordinates, .. }
            | Self::Connecting { coordinates, .. }
            | Self::Connected { coordinates, .. } => *coordinates,
            Self::Unavailable(_) | Self::Disconnecting { .. } | Self::Error(_) => None,
        }
    }
}
