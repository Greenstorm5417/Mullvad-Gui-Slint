use std::{env, path::PathBuf, time::SystemTime};

use async_trait::async_trait;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use crate::{
    controller::{DaemonApi, FeatureApi},
    model::{
        AccountStatus, AdvancedSettings, AppSettings, BooleanSetting, DeviceSummary, DnsBlocker,
        GeoCoordinate, MultihopMode, ObfuscationMode, RelayLocation, SplitTunnelState,
        TunnelStatus,
    },
};

#[allow(clippy::allow_attributes)]
pub mod proto {
    tonic::include_proto!("mullvad_daemon.management_interface");
}

use proto::{
    DeviceRemoval, ErrorState, GeographicLocationConstraint, LocationConstraint,
    QuantumResistantState, daemon_event, device_state, error_state, location_constraint,
    management_service_client::ManagementServiceClient, obfuscation_settings,
    quantum_resistant_state, relay_settings, tunnel_state::State, wireguard_constraints,
};

const DEFAULT_RPC_SOCKET_PATH: &str = "/var/run/mullvad-vpn";

#[derive(Clone, Debug)]
pub struct MullvadDaemon {
    socket_path: PathBuf,
}

impl Default for MullvadDaemon {
    fn default() -> Self {
        let socket_path = env::var_os("MULLVAD_RPC_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_RPC_SOCKET_PATH));
        Self { socket_path }
    }
}

#[async_trait]
impl DaemonApi for MullvadDaemon {
    async fn tunnel_status(&self) -> Result<TunnelStatus, String> {
        let mut client = self.connect_client().await?;
        client
            .get_tunnel_state(())
            .await
            .map(|response| to_app_status(response.into_inner()))
            .map_err(|error| format!("Could not read daemon state: {error}"))
    }

    async fn connect(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .connect_tunnel(())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not connect tunnel: {error}"))
    }

    async fn disconnect(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .disconnect_tunnel("gtk app disconnect".to_owned())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not disconnect tunnel: {error}"))
    }

    async fn reconnect(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .reconnect_tunnel(())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not reconnect tunnel: {error}"))
    }
}

#[async_trait]
impl FeatureApi for MullvadDaemon {
    async fn settings(&self) -> Result<AppSettings, String> {
        let mut client = self.connect_client().await?;
        let settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read settings: {error}"))?
            .into_inner();
        Ok(to_app_settings(settings))
    }

    async fn set_boolean_setting(
        &self,
        setting: BooleanSetting,
        enabled: bool,
    ) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let result = match setting {
            BooleanSetting::AllowLan => client.set_allow_lan(enabled).await,
            BooleanSetting::AutoConnect => client.set_auto_connect(enabled).await,
            BooleanSetting::EnableIpv6 => client.set_enable_ipv6(enabled).await,
            BooleanSetting::LockdownMode => client.set_lockdown_mode(enabled).await,
            BooleanSetting::ShowBetaReleases => client.set_show_beta_releases(enabled).await,
        };
        result
            .map(|_| ())
            .map_err(|error| format!("Could not update setting: {error}"))
    }

    async fn account_status(&self) -> Result<AccountStatus, String> {
        let mut client = self.connect_client().await?;
        let state = client
            .get_device(())
            .await
            .map_err(|error| format!("Could not read account state: {error}"))?
            .into_inner();

        let mut status = to_account_status(state)?;
        if let AccountStatus::LoggedIn {
            account_number,
            expiry,
            ..
        } = &mut status
            && let Ok(response) = client.get_account_data(account_number.clone()).await
        {
            *expiry = response.into_inner().expiry.map(format_expiry);
        }
        Ok(status)
    }

    async fn login(&self, account_number: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .login_account(account_number)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not log in: {error}"))
    }

    async fn logout(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .logout_account("gtk app logout".to_owned())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not log out: {error}"))
    }

    async fn create_account(&self) -> Result<String, String> {
        let mut client = self.connect_client().await?;
        client
            .create_new_account(())
            .await
            .map(|response| response.into_inner())
            .map_err(|error| format!("Could not create account: {error}"))
    }

    async fn submit_voucher(&self, voucher: String) -> Result<String, String> {
        let mut client = self.connect_client().await?;
        client
            .submit_voucher(voucher)
            .await
            .map(|response| {
                let response = response.into_inner();
                let days = response.seconds_added / 86_400;
                format!("Voucher accepted: {days} days added")
            })
            .map_err(|error| format!("Could not redeem voucher: {error}"))
    }

    async fn devices(&self, account_number: String) -> Result<Vec<DeviceSummary>, String> {
        let mut client = self.connect_client().await?;
        client
            .list_devices(account_number)
            .await
            .map(|response| {
                response
                    .into_inner()
                    .devices
                    .into_iter()
                    .map(|device| DeviceSummary {
                        id: device.id,
                        name: device.name,
                    })
                    .collect()
            })
            .map_err(|error| format!("Could not list devices: {error}"))
    }

    async fn remove_device(&self, account_number: String, device_id: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .remove_device(DeviceRemoval {
                account_number,
                device_id,
            })
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not remove device: {error}"))
    }

    async fn relay_locations(&self) -> Result<Vec<RelayLocation>, String> {
        let mut client = self.connect_client().await?;
        let relay_list = client
            .get_relay_locations(())
            .await
            .map_err(|error| format!("Could not read relay locations: {error}"))?
            .into_inner();
        Ok(to_relay_locations(relay_list))
    }

    async fn select_relay(&self, relay: RelayLocation) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read relay settings: {error}"))?
            .into_inner();
        let relay_settings = settings
            .relay_settings
            .as_mut()
            .ok_or_else(|| "Daemon omitted relay settings".to_owned())?;
        let normal = match relay_settings.endpoint.as_mut() {
            Some(relay_settings::Endpoint::Normal(normal)) => normal,
            _ => return Err("Custom tunnel configuration cannot select a relay".to_owned()),
        };
        normal.location = Some(LocationConstraint {
            r#type: Some(location_constraint::Type::Location(
                GeographicLocationConstraint {
                    country: relay.country_code,
                    city: relay.city_code,
                    hostname: relay.hostname,
                },
            )),
        });

        client
            .set_relay_settings(
                settings
                    .relay_settings
                    .expect("relay settings were checked"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not select relay: {error}"))
    }

    async fn split_tunnel_state(&self) -> Result<SplitTunnelState, String> {
        let mut client = self.connect_client().await?;
        let mut stream = client
            .get_split_tunnel_processes(())
            .await
            .map_err(|error| format!("Could not read split tunnel processes: {error}"))?
            .into_inner();
        let mut process_ids = Vec::new();
        while let Some(process_id) = stream
            .message()
            .await
            .map_err(|error| format!("Split tunnel process stream failed: {error}"))?
        {
            process_ids.push(process_id);
        }
        Ok(SplitTunnelState { process_ids })
    }

    async fn add_split_tunnel_process(&self, process_id: i32) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .add_split_tunnel_process(process_id)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not exclude process: {error}"))
    }

    async fn remove_split_tunnel_process(&self, process_id: i32) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .remove_split_tunnel_process(process_id)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not include process: {error}"))
    }

    async fn clear_split_tunnel_processes(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .clear_split_tunnel_processes(())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not clear excluded processes: {error}"))
    }

    async fn advanced_settings(&self) -> Result<AdvancedSettings, String> {
        let mut client = self.connect_client().await?;
        client
            .get_settings(())
            .await
            .map(|response| to_advanced_settings(response.into_inner()))
            .map_err(|error| format!("Could not read advanced settings: {error}"))
    }

    async fn set_dns_blocker(&self, blocker: DnsBlocker, enabled: bool) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read DNS settings: {error}"))?
            .into_inner();
        let dns_options = settings
            .tunnel_options
            .as_mut()
            .and_then(|options| options.dns_options.as_mut())
            .ok_or_else(|| "Daemon omitted DNS settings".to_owned())?;
        let default_options = dns_options
            .default_options
            .get_or_insert_with(Default::default);
        match blocker {
            DnsBlocker::Ads => default_options.block_ads = enabled,
            DnsBlocker::AdultContent => default_options.block_adult_content = enabled,
            DnsBlocker::Gambling => default_options.block_gambling = enabled,
            DnsBlocker::Malware => default_options.block_malware = enabled,
            DnsBlocker::SocialMedia => default_options.block_social_media = enabled,
            DnsBlocker::Trackers => default_options.block_trackers = enabled,
        }

        client
            .set_dns_options(dns_options.clone())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update DNS settings: {error}"))
    }

    async fn set_quantum_resistant(&self, enabled: bool) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let state = if enabled {
            quantum_resistant_state::State::On
        } else {
            quantum_resistant_state::State::Off
        };
        client
            .set_quantum_resistant_tunnel(QuantumResistantState {
                state: state.into(),
            })
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update quantum resistance: {error}"))
    }

    async fn set_daita(&self, enabled: bool) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .set_enable_daita(enabled)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update DAITA: {error}"))
    }

    async fn set_mtu(&self, mtu: Option<u32>) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .set_wireguard_mtu(mtu.unwrap_or(0))
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update WireGuard MTU: {error}"))
    }

    async fn set_multihop(&self, mode: MultihopMode) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read multihop settings: {error}"))?
            .into_inner();
        let relay_settings = settings
            .relay_settings
            .as_mut()
            .ok_or_else(|| "Daemon omitted relay settings".to_owned())?;
        let normal = match relay_settings.endpoint.as_mut() {
            Some(relay_settings::Endpoint::Normal(normal)) => normal,
            _ => return Err("Custom tunnel configuration cannot use multihop".to_owned()),
        };
        let constraints = normal
            .wireguard_constraints
            .get_or_insert_with(Default::default);
        constraints.multihop = match mode {
            MultihopMode::Auto => wireguard_constraints::Multihop::Auto,
            MultihopMode::Always => wireguard_constraints::Multihop::Always,
            MultihopMode::Never => wireguard_constraints::Multihop::Never,
        }
        .into();

        client
            .set_relay_settings(
                settings
                    .relay_settings
                    .expect("relay settings were checked"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update multihop: {error}"))
    }

    async fn set_obfuscation(&self, mode: ObfuscationMode) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read obfuscation settings: {error}"))?
            .into_inner();
        let obfuscation = settings
            .obfuscation_settings
            .as_mut()
            .ok_or_else(|| "Daemon omitted obfuscation settings".to_owned())?;
        obfuscation.selected_obfuscation = match mode {
            ObfuscationMode::Auto => obfuscation_settings::SelectedObfuscation::Auto,
            ObfuscationMode::Off => obfuscation_settings::SelectedObfuscation::Off,
            ObfuscationMode::WireguardPort => {
                obfuscation_settings::SelectedObfuscation::WireguardPort
            }
            ObfuscationMode::UdpOverTcp => obfuscation_settings::SelectedObfuscation::Udp2tcp,
            ObfuscationMode::Shadowsocks => obfuscation_settings::SelectedObfuscation::Shadowsocks,
            ObfuscationMode::Quic => obfuscation_settings::SelectedObfuscation::Quic,
            ObfuscationMode::Lwo => obfuscation_settings::SelectedObfuscation::Lwo,
        }
        .into();

        client
            .set_obfuscation_settings(*obfuscation)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update obfuscation: {error}"))
    }
}

impl MullvadDaemon {
    pub fn at_socket(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub async fn listen_tunnel_statuses(
        &self,
        sender: async_channel::Sender<TunnelStatus>,
    ) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut events = client
            .events_listen(())
            .await
            .map_err(|error| format!("Could not subscribe to daemon events: {error}"))?
            .into_inner();

        while let Some(event) = events
            .message()
            .await
            .map_err(|error| format!("Daemon event stream failed: {error}"))?
        {
            if let Some(daemon_event::Event::TunnelState(state)) = event.event
                && sender.send(to_app_status(state)).await.is_err()
            {
                return Ok(());
            }
        }

        Err("Daemon event stream closed".to_owned())
    }

    async fn connect_client(&self) -> Result<ManagementServiceClient<Channel>, String> {
        let socket_path = self.socket_path.clone();
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(service_fn(move |_: Uri| {
                let socket_path = socket_path.clone();
                async move { UnixStream::connect(socket_path).await.map(TokioIo::new) }
            }))
            .await
            .map_err(|error| format!("Could not connect to Mullvad daemon: {error}"))?;

        Ok(ManagementServiceClient::new(channel))
    }
}

fn to_app_status(state: proto::TunnelState) -> TunnelStatus {
    match state.state {
        Some(State::Disconnected(disconnected)) => {
            let (location, coordinates) = location_data(disconnected.disconnected_location);
            TunnelStatus::Disconnected {
                location,
                coordinates,
            }
        }
        Some(State::Connecting(connecting)) => {
            let location = connecting.relay_info.and_then(|relay| relay.location);
            let (location, coordinates) = location_data(location);
            TunnelStatus::Connecting {
                location,
                coordinates,
            }
        }
        Some(State::Connected(connected)) => {
            let location = connected.relay_info.and_then(|relay| relay.location);
            let (location, coordinates) = location_data(location);
            TunnelStatus::Connected {
                location,
                coordinates,
            }
        }
        Some(State::Disconnecting(_)) => TunnelStatus::Disconnecting,
        Some(State::Error(error)) => TunnelStatus::Error(format_error(error.error_state)),
        None => TunnelStatus::Error("Daemon returned an empty tunnel state".to_owned()),
    }
}

fn to_app_settings(settings: proto::Settings) -> AppSettings {
    let enable_ipv6 = settings
        .tunnel_options
        .as_ref()
        .is_some_and(|options| options.enable_ipv6);

    AppSettings {
        allow_lan: settings.allow_lan,
        auto_connect: settings.auto_connect,
        enable_ipv6,
        lockdown_mode: settings.lockdown_mode,
        show_beta_releases: settings.show_beta_releases,
    }
}

fn to_advanced_settings(settings: proto::Settings) -> AdvancedSettings {
    let tunnel_options = settings.tunnel_options.unwrap_or_default();
    let dns = tunnel_options
        .dns_options
        .and_then(|options| options.default_options)
        .unwrap_or_default();
    let quantum_resistant = tunnel_options
        .quantum_resistant
        .and_then(|state| quantum_resistant_state::State::try_from(state.state).ok())
        == Some(quantum_resistant_state::State::On);
    let daita = tunnel_options
        .daita
        .is_some_and(|settings| settings.enabled);
    let multihop = settings
        .relay_settings
        .and_then(|settings| settings.endpoint)
        .and_then(|endpoint| match endpoint {
            relay_settings::Endpoint::Normal(normal) => normal.wireguard_constraints,
            relay_settings::Endpoint::Custom(_) => None,
        })
        .and_then(|constraints| {
            wireguard_constraints::Multihop::try_from(constraints.multihop).ok()
        })
        .map(|mode| match mode {
            wireguard_constraints::Multihop::Auto => MultihopMode::Auto,
            wireguard_constraints::Multihop::Always => MultihopMode::Always,
            wireguard_constraints::Multihop::Never => MultihopMode::Never,
        })
        .unwrap_or(MultihopMode::Auto);
    let obfuscation = settings
        .obfuscation_settings
        .and_then(|settings| {
            obfuscation_settings::SelectedObfuscation::try_from(settings.selected_obfuscation).ok()
        })
        .map(|mode| match mode {
            obfuscation_settings::SelectedObfuscation::Auto => ObfuscationMode::Auto,
            obfuscation_settings::SelectedObfuscation::Off => ObfuscationMode::Off,
            obfuscation_settings::SelectedObfuscation::WireguardPort => {
                ObfuscationMode::WireguardPort
            }
            obfuscation_settings::SelectedObfuscation::Udp2tcp => ObfuscationMode::UdpOverTcp,
            obfuscation_settings::SelectedObfuscation::Shadowsocks => ObfuscationMode::Shadowsocks,
            obfuscation_settings::SelectedObfuscation::Quic => ObfuscationMode::Quic,
            obfuscation_settings::SelectedObfuscation::Lwo => ObfuscationMode::Lwo,
        })
        .unwrap_or(ObfuscationMode::Auto);

    AdvancedSettings {
        block_ads: dns.block_ads,
        block_adult_content: dns.block_adult_content,
        block_gambling: dns.block_gambling,
        block_malware: dns.block_malware,
        block_social_media: dns.block_social_media,
        block_trackers: dns.block_trackers,
        daita,
        mtu: tunnel_options.mtu,
        multihop,
        obfuscation,
        quantum_resistant,
    }
}

fn to_account_status(state: proto::DeviceState) -> Result<AccountStatus, String> {
    match device_state::State::try_from(state.state).ok() {
        Some(device_state::State::LoggedIn) => {
            let account = state
                .device
                .ok_or_else(|| "Daemon omitted logged-in device details".to_owned())?;
            let device_name = account
                .device
                .map(|device| device.name)
                .unwrap_or_else(|| "Unknown device".to_owned());
            Ok(AccountStatus::LoggedIn {
                account_number: account.account_number,
                device_name,
                expiry: None,
            })
        }
        Some(device_state::State::Revoked) => Ok(AccountStatus::Revoked),
        Some(device_state::State::LoggedOut) | None => Ok(AccountStatus::LoggedOut),
    }
}

fn format_expiry(expiry: prost_types::Timestamp) -> String {
    let now = SystemTime::UNIX_EPOCH
        .elapsed()
        .map_or(0, |duration| duration.as_secs() as i64);
    let remaining_seconds = expiry.seconds.saturating_sub(now);
    if remaining_seconds <= 0 {
        "Expired".to_owned()
    } else {
        let days = (remaining_seconds + 86_399) / 86_400;
        format!("{days} days remaining")
    }
}

fn to_relay_locations(relay_list: proto::RelayList) -> Vec<RelayLocation> {
    let mut locations = Vec::new();
    for country in relay_list.countries {
        locations.push(RelayLocation {
            label: country.name.clone(),
            country_code: country.code.clone(),
            city_code: None,
            hostname: None,
        });
        for city in country.cities {
            locations.push(RelayLocation {
                label: format!("{}, {}", city.name, country.name),
                country_code: country.code.clone(),
                city_code: Some(city.code.clone()),
                hostname: None,
            });
            for relay in city.relays.into_iter().filter(|relay| relay.active) {
                locations.push(RelayLocation {
                    label: format!("{} - {}, {}", relay.hostname, city.name, country.name),
                    country_code: country.code.clone(),
                    city_code: Some(city.code.clone()),
                    hostname: Some(relay.hostname),
                });
            }
        }
    }
    locations
}

fn format_location(location: &proto::GeoIpLocation) -> String {
    match &location.city {
        Some(city) if !city.is_empty() => format!("{city}, {}", location.country.to_uppercase()),
        _ => location.country.to_uppercase(),
    }
}

fn location_data(
    location: Option<proto::GeoIpLocation>,
) -> (Option<String>, Option<GeoCoordinate>) {
    let label = location.as_ref().map(format_location);
    let coordinates = location.map(|location| GeoCoordinate {
        latitude: location.latitude,
        longitude: location.longitude,
    });
    (label, coordinates)
}

fn format_error(error: Option<ErrorState>) -> String {
    error
        .and_then(|error| error_state::Cause::try_from(error.cause).ok())
        .map(|cause| format!("{cause:?}"))
        .unwrap_or_else(|| "Unknown daemon error".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_connected_state_and_location() {
        let status = to_app_status(proto::TunnelState {
            state: Some(State::Connected(proto::tunnel_state::Connected {
                relay_info: Some(proto::TunnelStateRelayInfo {
                    tunnel_endpoint: None,
                    location: Some(proto::GeoIpLocation {
                        city: Some("Gothenburg".to_owned()),
                        country: "se".to_owned(),
                        ..Default::default()
                    }),
                }),
                feature_indicators: None,
            })),
        });

        assert_eq!(
            status,
            TunnelStatus::Connected {
                location: Some("Gothenburg, SE".to_owned()),
                coordinates: Some(GeoCoordinate {
                    latitude: 0.0,
                    longitude: 0.0,
                }),
            }
        );
    }

    #[test]
    fn maps_account_and_primary_settings() {
        let account = to_account_status(proto::DeviceState {
            state: device_state::State::LoggedIn.into(),
            device: Some(proto::AccountAndDevice {
                account_number: "1234123412341234".to_owned(),
                device: Some(proto::Device {
                    name: "Test Device".to_owned(),
                    ..Default::default()
                }),
            }),
        })
        .unwrap();
        assert_eq!(
            account,
            AccountStatus::LoggedIn {
                account_number: "1234123412341234".to_owned(),
                device_name: "Test Device".to_owned(),
                expiry: None,
            }
        );

        let settings = to_app_settings(proto::Settings {
            allow_lan: true,
            auto_connect: true,
            lockdown_mode: true,
            show_beta_releases: true,
            tunnel_options: Some(proto::TunnelOptions {
                enable_ipv6: true,
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(
            settings,
            AppSettings {
                allow_lan: true,
                auto_connect: true,
                enable_ipv6: true,
                lockdown_mode: true,
                show_beta_releases: true,
            }
        );
    }

    #[test]
    fn flattens_active_relay_locations() {
        let locations = to_relay_locations(proto::RelayList {
            countries: vec![proto::RelayListCountry {
                name: "Sweden".to_owned(),
                code: "se".to_owned(),
                cities: vec![proto::RelayListCity {
                    name: "Gothenburg".to_owned(),
                    code: "got".to_owned(),
                    relays: vec![
                        proto::Relay {
                            hostname: "se-got-wg-001".to_owned(),
                            active: true,
                            ..Default::default()
                        },
                        proto::Relay {
                            hostname: "se-got-wg-002".to_owned(),
                            active: false,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
            }],
            endpoint_data: None,
        });

        assert_eq!(locations.len(), 3);
        assert_eq!(locations[2].hostname.as_deref(), Some("se-got-wg-001"));
    }

    #[test]
    fn maps_advanced_tunnel_settings() {
        let settings = to_advanced_settings(proto::Settings {
            tunnel_options: Some(proto::TunnelOptions {
                mtu: Some(1380),
                quantum_resistant: Some(proto::QuantumResistantState {
                    state: quantum_resistant_state::State::On.into(),
                }),
                daita: Some(proto::DaitaSettings { enabled: true }),
                dns_options: Some(proto::DnsOptions {
                    default_options: Some(proto::DefaultDnsOptions {
                        block_ads: true,
                        block_trackers: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            relay_settings: Some(proto::RelaySettings {
                endpoint: Some(relay_settings::Endpoint::Normal(
                    proto::NormalRelaySettings {
                        wireguard_constraints: Some(proto::WireguardConstraints {
                            multihop: wireguard_constraints::Multihop::Always.into(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )),
            }),
            obfuscation_settings: Some(proto::ObfuscationSettings {
                selected_obfuscation: obfuscation_settings::SelectedObfuscation::Quic.into(),
                ..Default::default()
            }),
            ..Default::default()
        });

        assert!(settings.block_ads);
        assert!(settings.block_trackers);
        assert!(settings.quantum_resistant);
        assert!(settings.daita);
        assert_eq!(settings.mtu, Some(1380));
        assert_eq!(settings.multihop, MultihopMode::Always);
        assert_eq!(settings.obfuscation, ObfuscationMode::Quic);
    }
}
