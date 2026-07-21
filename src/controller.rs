use async_trait::async_trait;

use crate::model::{
    AccountStatus, AdvancedSettings, AppSettings, BooleanSetting, DeviceSummary, DnsBlocker,
    MultihopMode, ObfuscationMode, RelayLocation, SplitTunnelState, TunnelStatus,
};

#[async_trait]
pub trait DaemonApi: Send + Sync {
    async fn tunnel_status(&self) -> Result<TunnelStatus, String>;
    async fn connect(&self) -> Result<(), String>;
    async fn disconnect(&self) -> Result<(), String>;
    async fn reconnect(&self) -> Result<(), String>;
}

#[async_trait]
pub trait FeatureApi: Send + Sync {
    async fn settings(&self) -> Result<AppSettings, String>;
    async fn set_boolean_setting(
        &self,
        setting: BooleanSetting,
        enabled: bool,
    ) -> Result<(), String>;
    async fn account_status(&self) -> Result<AccountStatus, String>;
    async fn login(&self, account_number: String) -> Result<(), String>;
    async fn logout(&self) -> Result<(), String>;
    async fn create_account(&self) -> Result<String, String>;
    async fn submit_voucher(&self, voucher: String) -> Result<String, String>;
    async fn devices(&self, account_number: String) -> Result<Vec<DeviceSummary>, String>;
    async fn remove_device(&self, account_number: String, device_id: String) -> Result<(), String>;
    async fn relay_locations(&self) -> Result<Vec<RelayLocation>, String>;
    async fn select_relay(&self, relay: RelayLocation) -> Result<(), String>;
    async fn split_tunnel_state(&self) -> Result<SplitTunnelState, String>;
    async fn add_split_tunnel_process(&self, process_id: i32) -> Result<(), String>;
    async fn remove_split_tunnel_process(&self, process_id: i32) -> Result<(), String>;
    async fn clear_split_tunnel_processes(&self) -> Result<(), String>;
    async fn advanced_settings(&self) -> Result<AdvancedSettings, String>;
    async fn set_dns_blocker(&self, blocker: DnsBlocker, enabled: bool) -> Result<(), String>;
    async fn set_quantum_resistant(&self, enabled: bool) -> Result<(), String>;
    async fn set_daita(&self, enabled: bool) -> Result<(), String>;
    async fn set_mtu(&self, mtu: Option<u32>) -> Result<(), String>;
    async fn set_multihop(&self, mode: MultihopMode) -> Result<(), String>;
    async fn set_obfuscation(&self, mode: ObfuscationMode) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaemonCommand {
    Refresh,
    Connect,
    Disconnect,
    Reconnect,
}

pub struct Controller<D> {
    daemon: D,
}

impl<D> Controller<D>
where
    D: DaemonApi,
{
    pub fn new(daemon: D) -> Self {
        Self { daemon }
    }

    pub async fn execute(&self, command: DaemonCommand) -> TunnelStatus {
        let command_result = match command {
            DaemonCommand::Refresh => Ok(()),
            DaemonCommand::Connect => self.daemon.connect().await,
            DaemonCommand::Disconnect => self.daemon.disconnect().await,
            DaemonCommand::Reconnect => self.daemon.reconnect().await,
        };

        if let Err(error) = command_result {
            return TunnelStatus::Error(error);
        }

        self.daemon
            .tunnel_status()
            .await
            .unwrap_or_else(TunnelStatus::Unavailable)
    }
}
