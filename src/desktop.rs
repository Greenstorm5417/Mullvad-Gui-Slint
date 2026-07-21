use std::{env, fs, path::PathBuf, time::Duration};

use async_channel::{Receiver, Sender};
use ksni::{TrayMethods, menu::StandardItem};
use notify_rust::{Hint, Notification, Timeout};

use crate::model::{ConnectionDetails, TunnelStatus};

const AUTOSTART_FILE: &str = "mullvad-gui-slint.desktop";
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
        "mullvad-gui-slint".to_owned()
    }

    fn title(&self) -> String {
        "Mullvad-Gui-Slint".to_owned()
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
            title: "Mullvad-Gui-Slint".to_owned(),
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
            tray_item("Open Mullvad-Gui-Slint", true, DesktopCommand::Show),
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
        TunnelStatus::Connecting { .. }
        | TunnelStatus::Disconnecting { .. }
        | TunnelStatus::Error(_) => 10,
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
    env::var_os("MULLVAD_GUI_SLINT_ASSET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let executable_relative = env::current_exe()
                .ok()
                .and_then(|path| path.parent()?.parent().map(PathBuf::from))
                .map(|prefix| prefix.join("share/mullvad-gui-slint/tray"));
            let installed = executable_relative
                .filter(|path| path.is_dir())
                .or_else(|| {
                    let path = PathBuf::from("/usr/share/mullvad-gui-slint/tray");
                    path.is_dir().then_some(path)
                })
                .unwrap_or_else(|| PathBuf::from("/usr/share/mullvad-gui-slint/tray"));
            if installed.is_dir() {
                installed
            } else {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/images/menubar-icons/linux")
            }
        })
}

/// Mirrors the upstream Electron client's notification bodies exactly (see
/// `shared/notifications/{connected,connecting,disconnected,reconnecting,
/// daemon-disconnected}.ts`): the OS notification's title is just the app
/// name, and each tunnel state has one fixed message using the exit relay's
/// hostname (not the friendly display location). A plain (non-reconnect)
/// "disconnecting" transition has no notification provider upstream, so we
/// show nothing for it either.
pub fn notify_tunnel_status(status: &TunnelStatus) {
    let body = match status {
        TunnelStatus::Connected { details, .. } => match hostname(details) {
            Some(hostname) => format!("Connected to {hostname}"),
            None => "Connected".to_owned(),
        },
        TunnelStatus::Connecting { details, .. } => match hostname(details) {
            Some(hostname) => format!("Connecting to {hostname}"),
            None => "Connecting".to_owned(),
        },
        TunnelStatus::Disconnected { .. } => "Disconnected and unsecure".to_owned(),
        TunnelStatus::Disconnecting {
            reconnecting: true, ..
        } => "Reconnecting".to_owned(),
        TunnelStatus::Disconnecting {
            reconnecting: false,
        } => return,
        TunnelStatus::Unavailable(_) => {
            "Connection might be unsecured. App lost contact with system service, please troubleshoot."
                .to_owned()
        }
        TunnelStatus::Error(message) => message.clone(),
    };
    let icon = tray_notification_icon(status);
    let _ = Notification::new()
        .appname("Mullvad-Gui-Slint")
        .summary("Mullvad-Gui-Slint")
        .body(&body)
        .icon(icon.to_string_lossy().as_ref())
        .hint(Hint::DesktopEntry("mullvad-gui-slint".to_owned()))
        .timeout(Timeout::Milliseconds(5_000))
        .show();
}

fn hostname(details: &Option<ConnectionDetails>) -> Option<&str> {
    details.as_ref()?.hostname.as_deref()
}

fn tray_notification_icon(status: &TunnelStatus) -> PathBuf {
    let frame = target_frame(status);
    let notification = tray_asset_dir().join(format!("lock-{frame}_notification.png"));
    if notification.is_file() {
        notification
    } else if tray_asset_dir().join(format!("lock-{frame}.png")).is_file() {
        tray_asset_dir().join(format!("lock-{frame}.png"))
    } else {
        PathBuf::from("mullvad-gui-slint")
    }
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

pub fn animate_map_enabled() -> bool {
    read_preferences().animate_map
}

pub fn set_animate_map_enabled(enabled: bool) -> Result<(), String> {
    let mut preferences = read_preferences();
    preferences.animate_map = enabled;
    write_preferences(preferences)
}

pub fn language() -> String {
    read_preferences().language
}

pub fn set_language(language: &str) -> Result<(), String> {
    let mut preferences = read_preferences();
    preferences.language = language.to_owned();
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
    let escaped_executable = desktop_exec_quote(&executable.to_string_lossy());
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Mullvad-Gui-Slint\nComment=Secure your connection with Mullvad VPN\nExec=\"{escaped_executable}\" --background\nIcon=mullvad-gui-slint\nTerminal=false\nCategories=Network;\nStartupNotify=false\nX-GNOME-Autostart-enabled=true\n"
    );
    fs::write(path, entry).map_err(|error| format!("Could not enable autostart: {error}"))
}

fn desktop_exec_quote(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn autostart_path() -> Option<PathBuf> {
    Some(config_dir()?.join("autostart").join(AUTOSTART_FILE))
}

fn config_dir() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

#[derive(Clone)]
struct DesktopPreferences {
    notifications: bool,
    monochromatic: bool,
    animate_map: bool,
    language: String,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            notifications: true,
            monochromatic: false,
            animate_map: true,
            // Matches upstream's SYSTEM_PREFERRED_LOCALE_KEY default (gui-settings.ts) —
            // a fresh install should default to "System default", not English.
            language: "system".to_owned(),
        }
    }
}

fn read_preferences() -> DesktopPreferences {
    let Some(path) = config_dir().map(|path| path.join("mullvad-gui-slint").join(PREFERENCES_FILE))
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
            Some(("animate_map", "true")) => preferences.animate_map = true,
            Some(("animate_map", "false")) => preferences.animate_map = false,
            Some(("language", value)) if !value.is_empty() => {
                preferences.language = value.to_owned();
            }
            _ => {}
        }
    }
    preferences
}

fn write_preferences(preferences: DesktopPreferences) -> Result<(), String> {
    let path = config_dir()
        .map(|path| path.join("mullvad-gui-slint").join(PREFERENCES_FILE))
        .ok_or_else(|| "No XDG configuration directory".to_owned())?;
    let parent = path
        .parent()
        .ok_or_else(|| "Invalid preferences path".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create preferences directory: {error}"))?;
    fs::write(
        path,
        format!(
            "notifications={}\nmonochromatic={}\nanimate_map={}\nlanguage={}\n",
            preferences.notifications,
            preferences.monochromatic,
            preferences.animate_map,
            preferences.language
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
                details: None,
            }),
            9
        );
        assert_eq!(
            target_frame(&TunnelStatus::Disconnecting {
                reconnecting: false
            }),
            10
        );
    }

    #[test]
    fn desktop_preferences_use_upstream_defaults_and_parse_saved_values() {
        let defaults = parse_preferences("");
        assert!(defaults.notifications);
        assert!(!defaults.monochromatic);
        assert!(defaults.animate_map);
        assert_eq!(defaults.language, "system");

        let saved = parse_preferences(
            "notifications=false\nmonochromatic=true\nanimate_map=false\nlanguage=sv\n",
        );
        assert!(!saved.notifications);
        assert!(saved.monochromatic);
        assert!(!saved.animate_map);
        assert_eq!(saved.language, "sv");
    }

    #[test]
    fn desktop_entry_exec_escapes_reserved_characters() {
        assert_eq!(
            desktop_exec_quote(r#"/opt/Mullvad $Test/`app`\"name"#),
            r#"/opt/Mullvad \$Test/\`app\`\\\"name"#
        );
    }
}
