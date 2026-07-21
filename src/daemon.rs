use std::{env, path::PathBuf, time::SystemTime};

use async_trait::async_trait;
use chrono::{Local, TimeZone};
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint, Uri};
use tower::service_fn;

use crate::{
    controller::{DaemonApi, FeatureApi},
    model::{
        AccountExpiry, AccountStatus, AdvancedSettings, ApiAccessMethodSummary, AppSettings,
        BooleanSetting, ConnectionDetails, CustomListSummary, DeviceSummary, DnsBlocker,
        GeoCoordinate, IpVersionMode, LocationSettings, MultihopMode, ObfuscationMode,
        OwnershipFilter, RelayLocation, RelayOverride, RelayRole, SplitTunnelState, TunnelStatus,
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
            .disconnect_tunnel("mullvad-gui-slint app disconnect".to_owned())
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
            .logout_account("mullvad-gui-slint app logout".to_owned())
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
                        created: device
                            .created
                            .and_then(|timestamp| {
                                Local
                                    .timestamp_opt(timestamp.seconds, timestamp.nanos as u32)
                                    .single()
                            })
                            .map(|created| created.format("%b %-d, %Y").to_string())
                            .unwrap_or_else(|| "Unknown".to_owned()),
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

    async fn select_automatic_relay(&self) -> Result<(), String> {
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
        normal.location = None;

        client
            .set_relay_settings(
                settings
                    .relay_settings
                    .expect("relay settings were checked"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not select an automatic relay: {error}"))
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
        let split_settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read split tunnel settings: {error}"))?
            .into_inner()
            .split_tunnel
            .unwrap_or_default();
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
        Ok(SplitTunnelState {
            enabled: split_settings.enable_exclusions,
            applications: split_settings.apps,
            process_ids,
        })
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

    pub async fn account_history(&self) -> Result<Option<String>, String> {
        let mut client = self.connect_client().await?;
        client
            .get_account_history(())
            .await
            .map(|response| response.into_inner().number)
            .map_err(|error| format!("Could not read account history: {error}"))
    }

    pub async fn clear_account_history(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .clear_account_history(())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not clear account history: {error}"))
    }

    pub async fn api_access_methods(&self) -> Result<Vec<ApiAccessMethodSummary>, String> {
        let mut client = self.connect_client().await?;
        let settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read API access methods: {error}"))?
            .into_inner()
            .api_access_methods
            .ok_or_else(|| "Daemon omitted API access methods".to_owned())?;
        let current_id = client
            .get_current_api_access_method(())
            .await
            .ok()
            .and_then(|response| response.into_inner().id)
            .map(|id| id.value);
        let mut methods = Vec::new();
        methods.extend(settings.direct.into_iter().map(|method| (method, false)));
        methods.extend(
            settings
                .mullvad_bridges
                .into_iter()
                .map(|method| (method, false)),
        );
        methods.extend(
            settings
                .encrypted_dns_proxy
                .into_iter()
                .map(|method| (method, false)),
        );
        methods.extend(settings.custom.into_iter().map(|method| (method, true)));
        Ok(methods
            .into_iter()
            .map(|(method, custom)| {
                let id = method.id.map(|id| id.value).unwrap_or_default();
                let (proxy_type, server, port, username, password, cipher) =
                    api_proxy_fields(method.access_method);
                ApiAccessMethodSummary {
                    in_use: current_id.as_deref() == Some(id.as_str()),
                    id,
                    name: method.name,
                    enabled: method.enabled,
                    custom,
                    proxy_type,
                    server,
                    port,
                    username,
                    password,
                    cipher,
                }
            })
            .collect())
    }

    pub async fn set_api_access_method_enabled(
        &self,
        id: String,
        enabled: bool,
    ) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read API access methods: {error}"))?
            .into_inner()
            .api_access_methods
            .ok_or_else(|| "Daemon omitted API access methods".to_owned())?;
        let mut methods = Vec::new();
        methods.extend(settings.direct);
        methods.extend(settings.mullvad_bridges);
        methods.extend(settings.encrypted_dns_proxy);
        methods.extend(settings.custom);
        let mut method = methods
            .into_iter()
            .find(|method| method.id.as_ref().is_some_and(|value| value.value == id))
            .ok_or_else(|| "API access method no longer exists".to_owned())?;
        method.enabled = enabled;
        client
            .update_api_access_method(method)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update API access method: {error}"))
    }

    pub async fn use_api_access_method(&self, id: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .set_api_access_method(proto::Uuid { value: id })
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not select API access method: {error}"))
    }

    pub async fn test_api_access_method(&self, id: String) -> Result<bool, String> {
        let mut client = self.connect_client().await?;
        client
            .test_api_access_method_by_id(proto::Uuid { value: id })
            .await
            .map(tonic::Response::into_inner)
            .map_err(|error| format!("Could not test API access method: {error}"))
    }

    pub async fn remove_api_access_method(&self, id: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .remove_api_access_method(proto::Uuid { value: id })
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not remove API access method: {error}"))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the arguments mirror the daemon's custom proxy editor fields"
    )]
    pub async fn save_api_access_method(
        &self,
        id: String,
        name: String,
        proxy_type: i32,
        server: String,
        port: u32,
        username: String,
        password: String,
        cipher: String,
    ) -> Result<(), String> {
        let proxy_method = match proxy_type {
            1 => proto::custom_proxy::ProxyMethod::Socks5remote(proto::Socks5Remote {
                ip: server,
                port,
                auth: (!username.is_empty() || !password.is_empty())
                    .then_some(proto::SocksAuth { username, password }),
            }),
            2 => proto::custom_proxy::ProxyMethod::Socks5local(proto::Socks5Local {
                remote_ip: server,
                remote_port: port,
                remote_transport_protocol: proto::TransportProtocol::Tcp.into(),
                local_port: port,
            }),
            _ => proto::custom_proxy::ProxyMethod::Shadowsocks(proto::Shadowsocks {
                ip: server,
                port,
                password,
                cipher: Some(proto::shadowsocks::Cipher { name: cipher }),
            }),
        };
        let access_method = Some(proto::AccessMethod {
            access_method: Some(proto::access_method::AccessMethod::Custom(
                proto::CustomProxy {
                    proxy_method: Some(proxy_method),
                },
            )),
        });
        let mut client = self.connect_client().await?;
        let result = if id.is_empty() {
            client
                .add_api_access_method(proto::NewAccessMethodSetting {
                    name,
                    enabled: true,
                    access_method,
                })
                .await
                .map(|_| ())
        } else {
            client
                .update_api_access_method(proto::AccessMethodSetting {
                    id: Some(proto::Uuid { value: id }),
                    name,
                    enabled: true,
                    access_method,
                })
                .await
                .map(|_| ())
        };
        result.map_err(|error| format!("Could not save API access method: {error}"))
    }

    pub async fn www_auth_token(&self) -> Result<String, String> {
        let mut client = self.connect_client().await?;
        client
            .get_www_auth_token(())
            .await
            .map(|response| response.into_inner())
            .map_err(|error| format!("Could not create web authentication token: {error}"))
    }

    pub async fn delete_account(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .delete_account(())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not delete account: {error}"))
    }

    pub async fn location_settings(&self) -> Result<LocationSettings, String> {
        let mut client = self.connect_client().await?;
        let relay_list = client
            .get_relay_locations(())
            .await
            .map_err(|error| format!("Could not read relay locations: {error}"))?
            .into_inner();
        let settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read location settings: {error}"))?
            .into_inner();
        let relays = to_relay_locations(relay_list);
        let mut providers = relays
            .iter()
            .filter_map(|relay| relay.provider.clone())
            .collect::<Vec<_>>();
        providers.sort();
        providers.dedup();

        let custom_lists = settings
            .custom_lists
            .map(|settings| settings.custom_lists)
            .unwrap_or_default()
            .into_iter()
            .map(|list| CustomListSummary {
                id: list.id,
                name: list.name,
                locations: list
                    .locations
                    .iter()
                    .filter_map(|location| relay_for_geographic(location, &relays))
                    .collect(),
            })
            .collect::<Vec<_>>();
        let recents_enabled = settings.recents.is_some();
        let recents = settings
            .recents
            .map(|recents| recents.recents)
            .unwrap_or_default()
            .into_iter()
            .flat_map(recent_constraints)
            .filter_map(|constraint| relay_for_constraint(&constraint, &relays, &custom_lists))
            .collect();
        let normal = settings
            .relay_settings
            .as_ref()
            .and_then(|relay_settings| relay_settings.endpoint.as_ref())
            .and_then(|endpoint| match endpoint {
                relay_settings::Endpoint::Normal(normal) => Some(normal),
                relay_settings::Endpoint::Custom(_) => None,
            });
        let selected_exit = normal
            .and_then(|normal| normal.location.as_ref())
            .and_then(|constraint| relay_for_constraint(constraint, &relays, &custom_lists));
        let selected_entry = normal
            .and_then(|normal| normal.wireguard_constraints.as_ref())
            .and_then(|wireguard| wireguard.entry_location.as_ref())
            .and_then(|constraint| relay_for_constraint(constraint, &relays, &custom_lists));

        Ok(LocationSettings {
            relays,
            recents,
            custom_lists,
            providers,
            recents_enabled,
            selected_entry,
            selected_exit,
        })
    }

    pub async fn select_relay_for_role(
        &self,
        relay: RelayLocation,
        role: RelayRole,
    ) -> Result<(), String> {
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
        let normal = normal_relay_settings(relay_settings)?;
        let constraint = relay_constraint(relay);
        match role {
            RelayRole::Exit => normal.location = Some(constraint),
            RelayRole::Entry => {
                normal
                    .wireguard_constraints
                    .get_or_insert_with(Default::default)
                    .entry_location = Some(constraint);
            }
        }
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

    pub async fn select_automatic_relay(&self, role: RelayRole) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read relay settings: {error}"))?
            .into_inner();
        let normal = normal_relay_settings(
            settings
                .relay_settings
                .as_mut()
                .ok_or_else(|| "Daemon omitted relay settings".to_owned())?,
        )?;
        let automatic = LocationConstraint { r#type: None };
        match role {
            RelayRole::Exit => normal.location = Some(automatic),
            RelayRole::Entry => {
                normal
                    .wireguard_constraints
                    .get_or_insert_with(Default::default)
                    .entry_location = Some(automatic);
            }
        }
        client
            .set_relay_settings(
                settings
                    .relay_settings
                    .expect("relay settings were checked"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not select automatic relay: {error}"))
    }

    pub async fn set_relay_filters(
        &self,
        role: RelayRole,
        ownership: OwnershipFilter,
        providers: Vec<String>,
    ) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read relay filters: {error}"))?
            .into_inner();
        let normal = normal_relay_settings(
            settings
                .relay_settings
                .as_mut()
                .ok_or_else(|| "Daemon omitted relay settings".to_owned())?,
        )?;
        let ownership = match ownership {
            OwnershipFilter::Any => proto::Ownership::Any,
            OwnershipFilter::MullvadOwned => proto::Ownership::MullvadOwned,
            OwnershipFilter::Rented => proto::Ownership::Rented,
        }
        .into();
        match role {
            RelayRole::Exit => {
                normal.ownership = ownership;
                normal.providers = providers;
            }
            RelayRole::Entry => {
                let constraints = normal
                    .wireguard_constraints
                    .get_or_insert_with(Default::default);
                constraints.entry_ownership = ownership;
                constraints.entry_providers = providers;
            }
        }
        client
            .set_relay_settings(
                settings
                    .relay_settings
                    .expect("relay settings were checked"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update relay filters: {error}"))
    }

    pub async fn set_enable_recents(&self, enabled: bool) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .set_enable_recents(enabled)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update recent locations: {error}"))
    }

    pub async fn create_custom_list(
        &self,
        name: String,
        locations: Vec<RelayLocation>,
    ) -> Result<String, String> {
        let mut client = self.connect_client().await?;
        client
            .create_custom_list(proto::NewCustomList {
                name,
                locations: locations.into_iter().map(geographic_constraint).collect(),
            })
            .await
            .map(|response| response.into_inner())
            .map_err(|error| format!("Could not create custom list: {error}"))
    }

    pub async fn delete_custom_list(&self, id: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .delete_custom_list(id)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not delete custom list: {error}"))
    }

    pub async fn update_custom_list(
        &self,
        id: String,
        name: String,
        locations: Vec<RelayLocation>,
    ) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .update_custom_list(proto::CustomList {
                id,
                name,
                locations: locations.into_iter().map(geographic_constraint).collect(),
            })
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update custom list: {error}"))
    }

    pub async fn set_custom_dns(
        &self,
        enabled: bool,
        addresses: Vec<String>,
    ) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read DNS settings: {error}"))?
            .into_inner();
        let dns = settings
            .tunnel_options
            .as_mut()
            .and_then(|options| options.dns_options.as_mut())
            .ok_or_else(|| "Daemon omitted DNS settings".to_owned())?;
        dns.state = if enabled {
            proto::dns_options::DnsState::Custom
        } else {
            proto::dns_options::DnsState::Default
        }
        .into();
        dns.custom_options = Some(proto::CustomDnsOptions { addresses });
        client
            .set_dns_options(dns.clone())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update custom DNS: {error}"))
    }

    pub async fn set_ip_version(&self, mode: IpVersionMode) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read IP version setting: {error}"))?
            .into_inner();
        let normal = normal_relay_settings(
            settings
                .relay_settings
                .as_mut()
                .ok_or_else(|| "Daemon omitted relay settings".to_owned())?,
        )?;
        normal
            .wireguard_constraints
            .get_or_insert_with(Default::default)
            .ip_version = match mode {
            IpVersionMode::Automatic => None,
            IpVersionMode::Ipv4 => Some(proto::IpVersion::V4.into()),
            IpVersionMode::Ipv6 => Some(proto::IpVersion::V6.into()),
        };
        client
            .set_relay_settings(
                settings
                    .relay_settings
                    .expect("relay settings were checked"),
            )
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update IP version: {error}"))
    }

    pub async fn set_allowed_ips(&self, values: Vec<String>) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .set_wireguard_allowed_ips(proto::AllowedIpsList { values })
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update WireGuard allowed IPs: {error}"))
    }

    pub async fn set_obfuscation_port(
        &self,
        mode: ObfuscationMode,
        port: Option<u32>,
    ) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        let mut settings = client
            .get_settings(())
            .await
            .map_err(|error| format!("Could not read anti-censorship settings: {error}"))?
            .into_inner();
        let obfuscation = settings
            .obfuscation_settings
            .as_mut()
            .ok_or_else(|| "Daemon omitted anti-censorship settings".to_owned())?;
        match mode {
            ObfuscationMode::WireguardPort => {
                obfuscation.wireguard_port =
                    Some(proto::obfuscation_settings::WireguardPort { port })
            }
            ObfuscationMode::UdpOverTcp => {
                obfuscation.udp2tcp = Some(proto::obfuscation_settings::Udp2TcpObfuscation { port })
            }
            ObfuscationMode::Shadowsocks => {
                obfuscation.shadowsocks = Some(proto::obfuscation_settings::Shadowsocks { port })
            }
            ObfuscationMode::Lwo => {
                obfuscation.lwo = Some(proto::obfuscation_settings::Lwo { port })
            }
            ObfuscationMode::Auto | ObfuscationMode::Off | ObfuscationMode::Quic => {}
        }
        client
            .set_obfuscation_settings(*obfuscation)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update anti-censorship port: {error}"))
    }

    pub async fn apply_json_settings(&self, json: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .apply_json_settings(json)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not import settings: {error}"))
    }

    pub async fn export_json_settings(&self) -> Result<String, String> {
        let mut client = self.connect_client().await?;
        client
            .export_json_settings(())
            .await
            .map(|response| response.into_inner())
            .map_err(|error| format!("Could not export settings: {error}"))
    }

    pub async fn clear_relay_overrides(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .clear_all_relay_overrides(())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not clear server IP overrides: {error}"))
    }

    pub async fn set_userspace_wireguard(&self, enabled: bool) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .set_userspace_wireguard(enabled)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update userspace WireGuard: {error}"))
    }

    pub async fn set_split_tunnel_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .set_split_tunnel_state(enabled)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not update split tunneling: {error}"))
    }

    pub async fn add_split_tunnel_app(&self, path: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .add_split_tunnel_app(path)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not exclude application: {error}"))
    }

    pub async fn remove_split_tunnel_app(&self, path: String) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .remove_split_tunnel_app(path)
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not include application: {error}"))
    }

    pub async fn clear_split_tunnel_apps(&self) -> Result<(), String> {
        let mut client = self.connect_client().await?;
        client
            .clear_split_tunnel_apps(())
            .await
            .map(|_| ())
            .map_err(|error| format!("Could not clear excluded applications: {error}"))
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
            let (location, coordinates) = location_data(disconnected.disconnected_location, false);
            TunnelStatus::Disconnected {
                location,
                coordinates,
            }
        }
        Some(State::Connecting(connecting)) => {
            let details = connection_details(
                connecting.relay_info.as_ref(),
                connecting.feature_indicators.as_ref(),
            );
            let location = connecting.relay_info.and_then(|relay| relay.location);
            let (location, coordinates) = location_data(location, true);
            TunnelStatus::Connecting {
                location,
                coordinates,
                details,
            }
        }
        Some(State::Connected(connected)) => {
            let details = connection_details(
                connected.relay_info.as_ref(),
                connected.feature_indicators.as_ref(),
            );
            let location = connected.relay_info.and_then(|relay| relay.location);
            let (location, coordinates) = location_data(location, true);
            TunnelStatus::Connected {
                location,
                coordinates,
                details,
            }
        }
        Some(State::Disconnecting(disconnecting)) => TunnelStatus::Disconnecting {
            reconnecting: disconnecting.after_disconnect() == proto::AfterDisconnect::Reconnect,
        },
        Some(State::Error(error)) => TunnelStatus::Error(format_error(error.error_state)),
        None => TunnelStatus::Error("Daemon returned an empty tunnel state".to_owned()),
    }
}

fn normal_relay_settings(
    relay_settings: &mut proto::RelaySettings,
) -> Result<&mut proto::NormalRelaySettings, String> {
    match relay_settings.endpoint.as_mut() {
        Some(relay_settings::Endpoint::Normal(normal)) => Ok(normal),
        _ => Err("Custom tunnel configuration cannot use relay constraints".to_owned()),
    }
}

fn geographic_constraint(relay: RelayLocation) -> GeographicLocationConstraint {
    GeographicLocationConstraint {
        country: relay.country_code,
        city: relay.city_code,
        hostname: relay.hostname,
    }
}

fn relay_constraint(relay: RelayLocation) -> LocationConstraint {
    let r#type = relay.custom_list_id.clone().map_or_else(
        || location_constraint::Type::Location(geographic_constraint(relay)),
        location_constraint::Type::CustomList,
    );
    LocationConstraint {
        r#type: Some(r#type),
    }
}

fn recent_constraints(recent: proto::Recent) -> Vec<LocationConstraint> {
    match recent.r#type {
        Some(proto::recent::Type::Singlehop(location)) => vec![location],
        Some(proto::recent::Type::Multihop(multihop)) => [multihop.entry, multihop.exit]
            .into_iter()
            .flatten()
            .collect(),
        None => Vec::new(),
    }
}

fn relay_for_constraint(
    constraint: &LocationConstraint,
    relays: &[RelayLocation],
    custom_lists: &[CustomListSummary],
) -> Option<RelayLocation> {
    match constraint.r#type.as_ref()? {
        location_constraint::Type::Location(location) => relay_for_geographic(location, relays),
        location_constraint::Type::CustomList(id) => custom_lists
            .iter()
            .find(|list| &list.id == id)
            .map(|list| RelayLocation {
                label: list.name.clone(),
                country_code: String::new(),
                city_code: None,
                hostname: None,
                custom_list_id: Some(list.id.clone()),
                depth: 0,
                provider: None,
                owned: None,
                daita: false,
            }),
    }
}

fn relay_for_geographic(
    location: &GeographicLocationConstraint,
    relays: &[RelayLocation],
) -> Option<RelayLocation> {
    relays
        .iter()
        .find(|relay| {
            relay.country_code == location.country
                && relay.city_code == location.city
                && relay.hostname == location.hostname
        })
        .cloned()
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
    let dns_options = tunnel_options.dns_options.clone().unwrap_or_default();
    let dns = dns_options.default_options.unwrap_or_default();
    let quantum_resistant = tunnel_options
        .quantum_resistant
        .and_then(|state| quantum_resistant_state::State::try_from(state.state).ok())
        == Some(quantum_resistant_state::State::On);
    let daita = tunnel_options
        .daita
        .is_some_and(|settings| settings.enabled);
    let wireguard_constraints = settings
        .relay_settings
        .as_ref()
        .and_then(|settings| settings.endpoint.as_ref())
        .and_then(|endpoint| match endpoint {
            relay_settings::Endpoint::Normal(normal) => normal.wireguard_constraints.as_ref(),
            relay_settings::Endpoint::Custom(_) => None,
        });
    let multihop = wireguard_constraints
        .and_then(|constraints| {
            wireguard_constraints::Multihop::try_from(constraints.multihop).ok()
        })
        .map(|mode| match mode {
            wireguard_constraints::Multihop::Auto => MultihopMode::Auto,
            wireguard_constraints::Multihop::Always => MultihopMode::Always,
            wireguard_constraints::Multihop::Never => MultihopMode::Never,
        })
        .unwrap_or(MultihopMode::Auto);
    let obfuscation_settings = settings.obfuscation_settings.unwrap_or_default();
    let obfuscation = obfuscation_settings::SelectedObfuscation::try_from(
        obfuscation_settings.selected_obfuscation,
    )
    .ok()
    .map(|mode| match mode {
        obfuscation_settings::SelectedObfuscation::Auto => ObfuscationMode::Auto,
        obfuscation_settings::SelectedObfuscation::Off => ObfuscationMode::Off,
        obfuscation_settings::SelectedObfuscation::WireguardPort => ObfuscationMode::WireguardPort,
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
        custom_dns_enabled: proto::dns_options::DnsState::try_from(dns_options.state).ok()
            == Some(proto::dns_options::DnsState::Custom),
        custom_dns_addresses: dns_options
            .custom_options
            .map(|options| options.addresses)
            .unwrap_or_default(),
        ip_version: wireguard_constraints
            .and_then(|constraints| constraints.ip_version)
            .and_then(|version| proto::IpVersion::try_from(version).ok())
            .map_or(IpVersionMode::Automatic, |version| match version {
                proto::IpVersion::V4 => IpVersionMode::Ipv4,
                proto::IpVersion::V6 => IpVersionMode::Ipv6,
            }),
        allowed_ips: wireguard_constraints
            .map(|constraints| constraints.allowed_ips.clone())
            .unwrap_or_default(),
        mtu: tunnel_options.mtu,
        multihop,
        obfuscation,
        wireguard_port: obfuscation_settings
            .wireguard_port
            .and_then(|settings| settings.port),
        udp_over_tcp_port: obfuscation_settings
            .udp2tcp
            .and_then(|settings| settings.port),
        shadowsocks_port: obfuscation_settings
            .shadowsocks
            .and_then(|settings| settings.port),
        lwo_port: obfuscation_settings.lwo.and_then(|settings| settings.port),
        quantum_resistant,
        relay_overrides: settings
            .relay_overrides
            .into_iter()
            .map(|relay| RelayOverride {
                hostname: relay.hostname,
                ipv4: relay.ipv4_addr_in,
                ipv6: relay.ipv6_addr_in,
            })
            .collect(),
        userspace_wireguard: tunnel_options.userspace,
    }
}

fn connection_details(
    relay_info: Option<&proto::TunnelStateRelayInfo>,
    indicators: Option<&proto::FeatureIndicators>,
) -> Option<ConnectionDetails> {
    let relay_info = relay_info?;
    let location = relay_info.location.as_ref();
    let endpoint = relay_info.tunnel_endpoint.as_ref();
    let inbound_endpoint = endpoint.map(connection_inbound_endpoint);
    Some(ConnectionDetails {
        hostname: location.and_then(|location| location.hostname.clone()),
        entry_hostname: location.and_then(|location| location.entry_hostname.clone()),
        in_address: inbound_endpoint.map(|(address, _)| address.to_owned()),
        out_ipv4: location.and_then(|location| location.ipv4.clone()),
        out_ipv6: location.and_then(|location| location.ipv6.clone()),
        protocol: inbound_endpoint.and_then(|(_, protocol)| {
            proto::TransportProtocol::try_from(protocol)
                .ok()
                .map(|protocol| format!("{protocol:?}").to_uppercase())
        }),
        features: indicators
            .map(|indicators| {
                indicators
                    .active_features
                    .iter()
                    .filter_map(|feature| proto::FeatureIndicator::try_from(*feature).ok())
                    .map(feature_label)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn connection_inbound_endpoint(endpoint: &proto::TunnelEndpoint) -> (&str, i32) {
    let obfuscation_endpoint = endpoint
        .obfuscation
        .as_ref()
        .and_then(|obfuscation| obfuscation.r#type.as_ref())
        .and_then(|kind| match kind {
            proto::obfuscation_info::Type::Single(endpoint) => endpoint.endpoint.as_ref(),
            proto::obfuscation_info::Type::Multiple(endpoints) => endpoints
                .obfuscators
                .first()
                .and_then(|endpoint| endpoint.endpoint.as_ref())
                .or(endpoints.direct.as_ref()),
        });

    if let Some(inbound) = obfuscation_endpoint.or(endpoint.entry_endpoint.as_ref()) {
        (inbound.address.as_str(), inbound.protocol)
    } else {
        (endpoint.address.as_str(), endpoint.protocol)
    }
}

fn feature_label(feature: proto::FeatureIndicator) -> String {
    format!("{feature:?}")
        .replace("Udp2Tcp", "UDP-over-TCP")
        .replace("Dns", "DNS ")
        .replace("Mtu", "MTU")
}

fn to_account_status(state: proto::DeviceState) -> Result<AccountStatus, String> {
    match device_state::State::try_from(state.state).ok() {
        Some(device_state::State::LoggedIn) => {
            let account = state
                .device
                .ok_or_else(|| "Daemon omitted logged-in device details".to_owned())?;
            let (device_id, device_name) = account
                .device
                .map(|device| (device.id, device.name))
                .unwrap_or_else(|| (String::new(), "Unknown device".to_owned()));
            Ok(AccountStatus::LoggedIn {
                account_number: account.account_number,
                device_id,
                device_name,
                expiry: None,
            })
        }
        Some(device_state::State::Revoked) => Ok(AccountStatus::Revoked),
        Some(device_state::State::LoggedOut) | None => Ok(AccountStatus::LoggedOut),
    }
}

fn format_expiry(expiry: prost_types::Timestamp) -> AccountExpiry {
    let now = SystemTime::UNIX_EPOCH
        .elapsed()
        .map_or(0, |duration| duration.as_secs() as i64);
    let remaining_seconds = expiry.seconds.saturating_sub(now);
    let remaining = if remaining_seconds <= 0 {
        "Expired".to_owned()
    } else {
        let days = remaining_seconds / 86_400;
        if days >= 730 {
            let years = days / 365;
            if years == 1 {
                "1 year".to_owned()
            } else {
                format!("{years} years")
            }
        } else if days == 1 {
            "1 day".to_owned()
        } else {
            format!("{days} days")
        }
    };
    let paid_until = chrono::DateTime::from_timestamp(expiry.seconds, expiry.nanos as u32)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %-d, %Y, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|| "Currently unavailable".to_owned());
    AccountExpiry {
        remaining,
        paid_until,
        expired: remaining_seconds <= 0,
        show_in_header: remaining_seconds > 3 * 86_400,
    }
}

fn api_proxy_fields(
    access_method: Option<proto::AccessMethod>,
) -> (i32, String, String, String, String, String) {
    use proto::{access_method, custom_proxy};

    let Some(access_method::AccessMethod::Custom(proxy)) =
        access_method.and_then(|method| method.access_method)
    else {
        return (
            0,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        );
    };
    match proxy.proxy_method {
        Some(custom_proxy::ProxyMethod::Shadowsocks(proxy)) => (
            0,
            proxy.ip,
            proxy.port.to_string(),
            String::new(),
            proxy.password,
            proxy.cipher.map(|cipher| cipher.name).unwrap_or_default(),
        ),
        Some(custom_proxy::ProxyMethod::Socks5remote(proxy)) => {
            let auth = proxy.auth.unwrap_or_default();
            (
                1,
                proxy.ip,
                proxy.port.to_string(),
                auth.username,
                auth.password,
                String::new(),
            )
        }
        Some(custom_proxy::ProxyMethod::Socks5local(proxy)) => (
            2,
            proxy.remote_ip,
            proxy.remote_port.to_string(),
            String::new(),
            String::new(),
            String::new(),
        ),
        None => (
            0,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
    }
}

/// Case-insensitive, numeric-aware comparison matching the upstream Electron
/// GUI's `label.localeCompare(label, locale, { numeric: true })` ordering, so
/// relay hostnames like `se-sto-wg-2` sort before `se-sto-wg-10`.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();
    loop {
        match (a_chars.peek(), b_chars.peek()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(ac), Some(bc)) if ac.is_ascii_digit() && bc.is_ascii_digit() => {
                let mut a_num = String::new();
                while let Some(c) = a_chars.peek().filter(|c| c.is_ascii_digit()) {
                    a_num.push(*c);
                    a_chars.next();
                }
                let mut b_num = String::new();
                while let Some(c) = b_chars.peek().filter(|c| c.is_ascii_digit()) {
                    b_num.push(*c);
                    b_chars.next();
                }
                let a_value: u64 = a_num.parse().unwrap_or(0);
                let b_value: u64 = b_num.parse().unwrap_or(0);
                match a_value.cmp(&b_value) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            }
            (Some(ac), Some(bc)) => {
                let (ac, bc) = (ac.to_ascii_lowercase(), bc.to_ascii_lowercase());
                match ac.cmp(&bc) {
                    std::cmp::Ordering::Equal => {
                        a_chars.next();
                        b_chars.next();
                    }
                    other => return other,
                }
            }
        }
    }
}

fn to_relay_locations(mut relay_list: proto::RelayList) -> Vec<RelayLocation> {
    relay_list
        .countries
        .sort_by(|a, b| natural_cmp(&a.name, &b.name));
    for country in &mut relay_list.countries {
        country.cities.sort_by(|a, b| natural_cmp(&a.name, &b.name));
        for city in &mut country.cities {
            city.relays
                .sort_by(|a, b| natural_cmp(&a.hostname, &b.hostname));
        }
    }
    let mut locations = Vec::new();
    for country in relay_list.countries {
        locations.push(RelayLocation {
            label: country.name.clone(),
            country_code: country.code.clone(),
            city_code: None,
            hostname: None,
            custom_list_id: None,
            depth: 0,
            provider: None,
            owned: None,
            daita: false,
        });
        for city in country.cities {
            locations.push(RelayLocation {
                label: format!("{}, {}", city.name, country.name),
                country_code: country.code.clone(),
                city_code: Some(city.code.clone()),
                hostname: None,
                custom_list_id: None,
                depth: 1,
                provider: None,
                owned: None,
                daita: false,
            });
            for relay in city.relays.into_iter().filter(|relay| relay.active) {
                locations.push(RelayLocation {
                    label: format!("{} - {}, {}", relay.hostname, city.name, country.name),
                    country_code: country.code.clone(),
                    city_code: Some(city.code.clone()),
                    hostname: Some(relay.hostname),
                    custom_list_id: None,
                    depth: 2,
                    provider: Some(relay.provider),
                    owned: Some(relay.owned),
                    daita: relay.endpoint_data.is_some_and(|endpoint| endpoint.daita),
                });
            }
        }
    }
    locations
}

fn format_location(location: &proto::GeoIpLocation, include_city: bool) -> String {
    match &location.city {
        Some(city) if include_city && !city.is_empty() => {
            format!("{}, {city}", location.country)
        }
        _ => location.country.clone(),
    }
}

fn location_data(
    location: Option<proto::GeoIpLocation>,
    include_city: bool,
) -> (Option<String>, Option<GeoCoordinate>) {
    let label = location
        .as_ref()
        .map(|location| format_location(location, include_city));
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
                location: Some("se, Gothenburg".to_owned()),
                coordinates: Some(GeoCoordinate {
                    latitude: 0.0,
                    longitude: 0.0,
                }),
                details: Some(ConnectionDetails {
                    hostname: None,
                    entry_hostname: None,
                    in_address: None,
                    out_ipv4: None,
                    out_ipv6: None,
                    protocol: None,
                    features: Vec::new(),
                }),
            }
        );
    }

    #[test]
    fn connection_details_prefers_outermost_inbound_endpoint() {
        let relay_info = proto::TunnelStateRelayInfo {
            tunnel_endpoint: Some(proto::TunnelEndpoint {
                address: "10.0.0.1:51820".to_owned(),
                protocol: proto::TransportProtocol::Udp.into(),
                entry_endpoint: Some(proto::Endpoint {
                    address: "10.0.0.2:51820".to_owned(),
                    protocol: proto::TransportProtocol::Udp.into(),
                }),
                obfuscation: Some(proto::ObfuscationInfo {
                    r#type: Some(proto::obfuscation_info::Type::Single(
                        proto::ObfuscationEndpoint {
                            endpoint: Some(proto::Endpoint {
                                address: "10.0.0.3:443".to_owned(),
                                protocol: proto::TransportProtocol::Tcp.into(),
                            }),
                            ..Default::default()
                        },
                    )),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let details = connection_details(Some(&relay_info), None).unwrap();
        assert_eq!(details.in_address.as_deref(), Some("10.0.0.3:443"));
        assert_eq!(details.protocol.as_deref(), Some("TCP"));
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
                device_id: String::new(),
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
    fn account_expiry_keeps_absolute_and_relative_presentations() {
        let expiry = format_expiry(prost_types::Timestamp {
            seconds: 0,
            nanos: 0,
        });
        assert!(expiry.expired);
        assert_eq!(expiry.remaining, "Expired");
        assert_ne!(expiry.paid_until, "Currently unavailable");
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
