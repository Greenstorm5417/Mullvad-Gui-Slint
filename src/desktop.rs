use std::{env, fs, path::PathBuf, time::Duration};

use async_channel::{Receiver, Sender};
use ksni::{TrayMethods, menu::StandardItem};
use notify_rust::{Notification, Timeout};

use crate::model::TunnelStatus;

const AUTOSTART_FILE: &str = "mullvad-gtk.desktop";
const PREFERENCES_FILE: &str = "preferences.conf";
const TRAY_FRAME_DELAY: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopCommand {
    Show,
    Connect,
    Reconnect,
    Disconnect,
    DisconnectAndQuit,
}

#[derive(Clone, Debug)]
pub enum TrayUpdate {
    Status(TunnelStatus),
    Monochromatic(bool),
}

struct MullvadTray {
    availability_sender: Sender<bool>,
    command_sender: Sender<DesktopCommand>,
    frame: u8,
    monochromatic: bool,
    status: TunnelStatus,
}

impl ksni::Tray for MullvadTray {
    fn id(&self) -> String {
        "mullvad-gtk".to_owned()
    }

    fn title(&self) -> String {
        "Mullvad VPN".to_owned()
    }

    fn icon_theme_path(&self) -> String {
        tray_asset_dir().to_string_lossy().into_owned()
    }

    fn icon_name(&self) -> String {
        let suffix = if self.monochromatic { "_white" } else { "" };
        format!("lock-{}{suffix}", self.frame)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Mullvad VPN".to_owned(),
            description: format!("{} - {}", self.status.headline(), self.status.detail()),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.command_sender.try_send(DesktopCommand::Show);
    }

    fn watcher_online(&self) {
        let _ = self.availability_sender.try_send(true);
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        let _ = self.availability_sender.try_send(false);
        true
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let connect_enabled = matches!(
            self.status,
            TunnelStatus::Disconnected { .. }
                | TunnelStatus::Error(_)
                | TunnelStatus::Unavailable(_)
        );
        let reconnect_enabled = matches!(self.status, TunnelStatus::Connected { .. });
        let disconnect_enabled = matches!(
            self.status,
            TunnelStatus::Connected { .. } | TunnelStatus::Connecting { .. }
        );

        vec![
            tray_item("Open Mullvad VPN", true, DesktopCommand::Show),
            ksni::MenuItem::Separator,
            tray_item("Connect", connect_enabled, DesktopCommand::Connect),
            tray_item("Reconnect", reconnect_enabled, DesktopCommand::Reconnect),
            tray_item("Disconnect", disconnect_enabled, DesktopCommand::Disconnect),
            ksni::MenuItem::Separator,
            tray_item(
                if disconnect_enabled {
                    "Disconnect && quit"
                } else {
                    "Quit"
                },
                true,
                DesktopCommand::DisconnectAndQuit,
            ),
        ]
    }
}

fn tray_item(label: &str, enabled: bool, command: DesktopCommand) -> ksni::MenuItem<MullvadTray> {
    StandardItem {
        label: label.to_owned(),
        enabled,
        activate: Box::new(move |tray: &mut MullvadTray| {
            let _ = tray.command_sender.try_send(command);
        }),
        ..Default::default()
    }
    .into()
}

pub fn start_tray(
    runtime: &tokio::runtime::Runtime,
    command_sender: Sender<DesktopCommand>,
    availability_sender: Sender<bool>,
) -> Sender<TrayUpdate> {
    let (status_sender, status_receiver) = async_channel::unbounded();
    runtime.spawn(run_tray(
        command_sender,
        status_receiver,
        availability_sender,
    ));
    status_sender
}

async fn run_tray(
    command_sender: Sender<DesktopCommand>,
    status_receiver: Receiver<TrayUpdate>,
    availability_sender: Sender<bool>,
) {
    let initial_status = TunnelStatus::Unavailable("Connecting to system service".to_owned());
    let tray = MullvadTray {
        availability_sender: availability_sender.clone(),
        command_sender,
        frame: target_frame(&initial_status),
        monochromatic: monochromatic_enabled(),
        status: initial_status,
    };
    let Ok(handle) = tray.spawn().await else {
        let _ = availability_sender.send(false).await;
        return;
    };
    let _ = availability_sender.send(true).await;

    while let Ok(update) = status_receiver.recv().await {
        let status = match update {
            TrayUpdate::Status(status) => status,
            TrayUpdate::Monochromatic(monochromatic) => {
                handle
                    .update(move |tray| tray.monochromatic = monochromatic)
                    .await;
                continue;
            }
        };
        let target = target_frame(&status);
        let current = handle
            .update({
                let status = status.clone();
                move |tray| {
                    tray.status = status;
                    tray.frame
                }
            })
            .await
            .unwrap_or(target);

        for frame in frames_between(current, target) {
            handle.update(move |tray| tray.frame = frame).await;
            tokio::time::sleep(TRAY_FRAME_DELAY).await;
        }
    }

    handle.shutdown().await;
    let _ = availability_sender.send(false).await;
}

fn target_frame(status: &TunnelStatus) -> u8 {
    match status {
        TunnelStatus::Connected { .. } => 9,
        TunnelStatus::Connecting { .. } | TunnelStatus::Disconnecting | TunnelStatus::Error(_) => {
            10
        }
        TunnelStatus::Disconnected { .. } | TunnelStatus::Unavailable(_) => 1,
    }
}

fn frames_between(current: u8, target: u8) -> Vec<u8> {
    if current < target {
        ((current + 1)..=target).collect()
    } else {
        (target..current).rev().collect()
    }
}

fn tray_asset_dir() -> PathBuf {
    env::var_os("MULLVAD_GTK_ASSET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let executable_relative = env::current_exe()
                .ok()
                .and_then(|path| path.parent()?.parent().map(PathBuf::from))
                .map(|prefix| prefix.join("share/mullvad-gtk/tray"));
            let installed = executable_relative
                .filter(|path| path.is_dir())
                .unwrap_or_else(|| PathBuf::from("/usr/share/mullvad-gtk/tray"));
            if installed.is_dir() {
                installed
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/images/menubar-icons/linux")
            }
        })
}

pub fn notify_tunnel_status(status: &TunnelStatus) {
    let body = match status {
        TunnelStatus::Connected { location, .. } => location
            .as_deref()
            .map_or("Your connection is secure", |location| location),
        TunnelStatus::Connecting { .. } => "Creating a secure connection",
        TunnelStatus::Disconnected { .. } => "Your connection is not secure",
        TunnelStatus::Disconnecting => "Closing the VPN tunnel",
        TunnelStatus::Error(message) | TunnelStatus::Unavailable(message) => message,
    };
    let _ = Notification::new()
        .appname("Mullvad VPN")
        .summary(status.headline())
        .body(body)
        .icon("mullvad-gtk")
        .timeout(Timeout::Milliseconds(5_000))
        .show();
}

pub fn autostart_enabled() -> bool {
    autostart_path().is_some_and(|path| path.is_file())
}

pub fn notifications_enabled() -> bool {
    read_preferences().notifications
}

pub fn set_notifications_enabled(enabled: bool) -> Result<(), String> {
    let mut preferences = read_preferences();
    preferences.notifications = enabled;
    write_preferences(preferences)
}

pub fn monochromatic_enabled() -> bool {
    read_preferences().monochromatic
}

pub fn set_monochromatic_enabled(enabled: bool) -> Result<(), String> {
    let mut preferences = read_preferences();
    preferences.monochromatic = enabled;
    write_preferences(preferences)
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let path = autostart_path().ok_or_else(|| "No XDG configuration directory".to_owned())?;
    if !enabled {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Could not disable autostart: {error}")),
        };
    }

    let executable = env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(env::current_exe)
        .map_err(|error| format!("Could not locate application executable: {error}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid autostart path".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create autostart directory: {error}"))?;
    let escaped_executable = executable.to_string_lossy().replace('"', "\\\"");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Mullvad-GTK\nComment=Secure your connection with Mullvad VPN\nExec=\"{escaped_executable}\" --background\nIcon=mullvad-gtk\nTerminal=false\nCategories=Network;Security;\nX-GNOME-Autostart-enabled=true\n"
    );
    fs::write(path, entry).map_err(|error| format!("Could not enable autostart: {error}"))
}

fn autostart_path() -> Option<PathBuf> {
    Some(config_dir()?.join("autostart").join(AUTOSTART_FILE))
}

fn config_dir() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

#[derive(Clone, Copy)]
struct DesktopPreferences {
    notifications: bool,
    monochromatic: bool,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            notifications: true,
            monochromatic: false,
        }
    }
}

fn read_preferences() -> DesktopPreferences {
    let Some(path) = config_dir().map(|path| path.join("mullvad-gtk").join(PREFERENCES_FILE))
    else {
        return DesktopPreferences::default();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return DesktopPreferences::default();
    };
    parse_preferences(&contents)
}

fn parse_preferences(contents: &str) -> DesktopPreferences {
    let mut preferences = DesktopPreferences::default();
    for line in contents.lines() {
        match line.split_once('=') {
            Some(("notifications", "false")) => preferences.notifications = false,
            Some(("notifications", "true")) => preferences.notifications = true,
            Some(("monochromatic", "true")) => preferences.monochromatic = true,
            Some(("monochromatic", "false")) => preferences.monochromatic = false,
            _ => {}
        }
    }
    preferences
}

fn write_preferences(preferences: DesktopPreferences) -> Result<(), String> {
    let path = config_dir()
        .map(|path| path.join("mullvad-gtk").join(PREFERENCES_FILE))
        .ok_or_else(|| "No XDG configuration directory".to_owned())?;
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid preferences path".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create preferences directory: {error}"))?;
    fs::write(
        path,
        format!(
            "notifications={}\nmonochromatic={}\n",
            preferences.notifications, preferences.monochromatic
        ),
    )
    .map_err(|error| format!("Could not save desktop preferences: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_frames_move_in_both_directions_without_repeating_the_start() {
        assert_eq!(frames_between(1, 4), vec![2, 3, 4]);
        assert_eq!(frames_between(4, 1), vec![3, 2, 1]);
        assert!(frames_between(4, 4).is_empty());
    }

    #[test]
    fn tunnel_states_select_upstream_lock_frames() {
        assert_eq!(
            target_frame(&TunnelStatus::Disconnected {
                location: None,
                coordinates: None,
            }),
            1
        );
        assert_eq!(
            target_frame(&TunnelStatus::Connected {
                location: None,
                coordinates: None,
            }),
            9
        );
        assert_eq!(target_frame(&TunnelStatus::Disconnecting), 10);
    }

    #[test]
    fn desktop_preferences_use_upstream_defaults_and_parse_saved_values() {
        let defaults = parse_preferences("");
        assert!(defaults.notifications);
        assert!(!defaults.monochromatic);

        let saved = parse_preferences("notifications=false\nmonochromatic=true\n");
        assert!(!saved.notifications);
        assert!(saved.monochromatic);
    }
}
