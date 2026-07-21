use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use mullvad_gtk::{
    controller::{Controller, DaemonApi, DaemonCommand, FeatureApi},
    daemon::MullvadDaemon,
    model::TunnelStatus,
};

#[derive(Clone)]
struct FakeDaemon {
    state: Arc<Mutex<TunnelStatus>>,
    calls: Arc<Mutex<Vec<DaemonCommand>>>,
}

impl FakeDaemon {
    fn disconnected() -> Self {
        Self {
            state: Arc::new(Mutex::new(TunnelStatus::Disconnected {
                location: Some("Gothenburg, SE".to_owned()),
                coordinates: None,
            })),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl DaemonApi for FakeDaemon {
    async fn tunnel_status(&self) -> Result<TunnelStatus, String> {
        Ok(self.state.lock().unwrap().clone())
    }

    async fn connect(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push(DaemonCommand::Connect);
        *self.state.lock().unwrap() = TunnelStatus::Connected {
            location: Some("Gothenburg, SE".to_owned()),
            coordinates: None,
        };
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push(DaemonCommand::Disconnect);
        *self.state.lock().unwrap() = TunnelStatus::Disconnected {
            location: Some("Gothenburg, SE".to_owned()),
            coordinates: None,
        };
        Ok(())
    }

    async fn reconnect(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push(DaemonCommand::Reconnect);
        *self.state.lock().unwrap() = TunnelStatus::Connecting {
            location: Some("Gothenburg, SE".to_owned()),
            coordinates: None,
        };
        Ok(())
    }
}

#[tokio::test]
async fn connect_and_disconnect_flow_uses_daemon_api() {
    let daemon = FakeDaemon::disconnected();
    let calls = Arc::clone(&daemon.calls);
    let controller = Controller::new(daemon);

    let connected = controller.execute(DaemonCommand::Connect).await;
    assert!(matches!(connected, TunnelStatus::Connected { .. }));

    let disconnected = controller.execute(DaemonCommand::Disconnect).await;
    assert!(matches!(disconnected, TunnelStatus::Disconnected { .. }));
    assert_eq!(
        *calls.lock().unwrap(),
        vec![DaemonCommand::Connect, DaemonCommand::Disconnect]
    );
}

#[tokio::test]
async fn reconnect_flow_uses_daemon_api() {
    let daemon = FakeDaemon::disconnected();
    let calls = Arc::clone(&daemon.calls);
    let controller = Controller::new(daemon);

    let reconnecting = controller.execute(DaemonCommand::Reconnect).await;

    assert!(matches!(reconnecting, TunnelStatus::Connecting { .. }));
    assert_eq!(*calls.lock().unwrap(), vec![DaemonCommand::Reconnect]);
}

#[tokio::test]
#[ignore = "requires the Mullvad daemon to be installed and running"]
async fn reads_state_from_live_daemon_socket() {
    let controller = Controller::new(MullvadDaemon::default());
    let status = controller.execute(DaemonCommand::Refresh).await;

    assert!(!matches!(status, TunnelStatus::Unavailable(_)));
}

#[tokio::test]
#[ignore = "requires the Mullvad daemon to be installed and running"]
async fn reads_settings_from_live_daemon_socket() {
    let settings = MullvadDaemon::default().settings().await;

    assert!(
        settings.is_ok(),
        "daemon settings request failed: {settings:?}"
    );
}
