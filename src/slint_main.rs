#![expect(
    clippy::as_underscore,
    reason = "Slint 1.17 generates inferred casts in include_modules"
)]

use std::{
    collections::HashSet,
    env, fs,
    net::IpAddr,
    path::{Path, PathBuf},
    process::{self, Command},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use mullvad_gui_slint::{
    controller::{DaemonApi, FeatureApi},
    daemon::MullvadDaemon,
    desktop::{self, DesktopCommand, TrayUpdate},
    model::{
        AccountStatus, BooleanSetting, ConnectionDetails, CustomListSummary, DnsBlocker,
        IpVersionMode, LocationSettings, MultihopMode, ObfuscationMode, OwnershipFilter,
        RelayLocation, TunnelStatus,
    },
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

mod map_render;

slint::include_modules!();

const GITHUB_RELEASES_API: &str =
    "https://api.github.com/repos/Greenstorm5417/Mullvad-Gui-Slint/releases?per_page=20";
const GITHUB_RELEASES_URL: &str = "https://github.com/Greenstorm5417/Mullvad-Gui-Slint/releases";

struct ReleaseInfo {
    version: semver::Version,
    url: String,
}

struct RelayUiState {
    relays: Vec<RelayLocation>,
    recent_count: usize,
    custom_count: usize,
    expanded: HashSet<usize>,
    query: String,
    recents_enabled: bool,
    location_type: i32,
    selected_entry: Option<RelayLocation>,
    selected_exit: Option<RelayLocation>,
    providers: Vec<String>,
    selected_providers: HashSet<String>,
    ownership_filter: i32,
    custom_lists: Vec<CustomListSummary>,
}

impl Default for RelayUiState {
    fn default() -> Self {
        Self {
            relays: Vec::new(),
            recent_count: 0,
            custom_count: 0,
            expanded: HashSet::new(),
            query: String::new(),
            recents_enabled: false,
            location_type: 1,
            selected_entry: None,
            selected_exit: None,
            providers: Vec::new(),
            selected_providers: HashSet::new(),
            ownership_filter: -1,
            custom_lists: Vec::new(),
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime"),
    );
    let daemon = Arc::new(MullvadDaemon::default());
    let window = MullvadWindow::new()?;
    window.set_app_version(env!("CARGO_PKG_VERSION").into());
    window.set_is_beta_version(is_beta_version(env!("CARGO_PKG_VERSION")));
    window.set_changelog_items(ModelRc::new(VecModel::from(
        include_str!("../CHANGELOG.md")
            .lines()
            .filter_map(|line| line.strip_prefix("- ").map(Into::into))
            .collect::<Vec<slint::SharedString>>(),
    )));
    let map_animator = Arc::new(Mutex::new(map_render::MapAnimator::new(
        mullvad_gui_slint::model::GeoCoordinate {
            latitude: 57.708_87,
            longitude: 11.974_56,
        },
    )));
    let (map_request_sender, map_request_receiver) = std::sync::mpsc::sync_channel(1);
    let (map_result_sender, map_result_receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("mullvad-map-renderer".to_owned())
        .spawn(move || {
            while let Ok(frame) = map_request_receiver.recv() {
                let pixels = map_render::render_map(frame);
                let _ = map_result_sender.send(pixels);
            }
        })
        .expect("map renderer thread");
    let map_timer = slint::Timer::default();
    {
        let window = window.as_weak();
        let map_animator = Arc::clone(&map_animator);
        let map_request_sender = map_request_sender.clone();
        let mut previous_frame = Instant::now();
        map_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(33),
            move || {
                let now = Instant::now();
                let delta = now.saturating_duration_since(previous_frame);
                previous_frame = now;
                let Some(window) = window.upgrade() else {
                    return;
                };
                if window.get_page().as_str() != "main" {
                    while map_result_receiver.try_recv().is_ok() {}
                    return;
                }
                if let Ok(pixels) = map_result_receiver.try_recv() {
                    window.set_disconnected_map(slint::Image::from_rgb8(pixels));
                }
                let mut animator = map_animator.lock().expect("map animator lock");
                if let Some(frame) = animator.frame(delta)
                    && map_request_sender.try_send(frame).is_err()
                {
                    animator.request_redraw();
                }
            },
        );
    }
    let account_refresh_timer = slint::Timer::default();
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        account_refresh_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(60 * 60),
            move || {
                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                runtime.spawn(async move {
                    if let Ok(status) = daemon.account_status().await {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                apply_account(&window, status);
                            }
                        });
                    }
                });
            },
        );
    }
    let relay_locations = Arc::new(Mutex::new(RelayUiState::default()));
    let arguments = std::env::args().collect::<Vec<_>>();
    let start_in_background = arguments.iter().any(|argument| argument == "--background");
    let initial_page = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--page="))
        .unwrap_or("main");
    let initial_settings_page = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--settings-page="))
        .and_then(|page| page.parse::<i32>().ok())
        .unwrap_or(0);
    let initial_account_page = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--account-page="))
        .unwrap_or("account");
    let initial_location_overlay = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix("--location-overlay="))
        .unwrap_or("");
    window.set_settings_page(initial_settings_page);
    window.set_account_page(initial_account_page.into());
    window.set_location_initial_overlay(initial_location_overlay.into());
    let (desktop_sender, desktop_receiver) = async_channel::unbounded();
    let (tray_available_sender, tray_available_receiver) = async_channel::unbounded();
    let tray_status_sender = desktop::start_tray(&runtime, desktop_sender, tray_available_sender);
    let tray_available = Arc::new(AtomicBool::new(false));
    {
        let tray_available = Arc::clone(&tray_available);
        runtime.spawn(async move {
            while let Ok(available) = tray_available_receiver.recv().await {
                tray_available.store(available, Ordering::Relaxed);
            }
        });
    }
    {
        let tray_available = Arc::clone(&tray_available);
        window.window().on_close_requested(move || {
            if !tray_available.load(Ordering::Relaxed) {
                let _ = slint::quit_event_loop();
            }
            slint::CloseRequestResponse::HideWindow
        });
    }

    {
        let window = window.as_weak();
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        window.upgrade().unwrap().on_connect_clicked(move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let should_disconnect =
                window.get_connected() || window.get_status().as_str() == "CONNECTING...";
            let window = window.as_weak();
            let daemon = Arc::clone(&daemon);
            runtime.spawn(async move {
                let result = if should_disconnect {
                    daemon.disconnect().await
                } else {
                    daemon.connect().await
                };
                if let Err(error) = result {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            window.set_detail(error.into());
                        }
                    });
                }
            });
        });
    }
    {
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_prepare_add_location(move |index| {
                let choices = usize::try_from(index).ok().map(|index| {
                    custom_list_choice_rows(
                        &relay_locations.lock().expect("relay location lock"),
                        index,
                    )
                });
                if let (Some(window), Some(choices)) = (window.upgrade(), choices) {
                    window.set_custom_list_choices(ModelRc::new(VecModel::from(choices)));
                }
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_add_location_to_custom_list(move |list_index, relay_index| {
                let update = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    update_custom_list_location(
                        &mut state,
                        usize::try_from(list_index).ok(),
                        usize::try_from(relay_index).ok(),
                        true,
                    )
                };
                let Some((list, _)) = update else {
                    return;
                };
                let daemon = Arc::clone(&daemon);
                let relay_locations = Arc::clone(&relay_locations);
                let window = window.clone();
                runtime.spawn(async move {
                    match daemon
                        .update_custom_list(list.id, list.name, list.locations)
                        .await
                    {
                        Ok(()) => reload_location_models(daemon, relay_locations, window).await,
                        Err(error) => show_window_error(window, error),
                    }
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        window.on_reconnect_clicked(move || {
            let daemon = Arc::clone(&daemon);
            runtime.spawn(async move {
                let _ = daemon.reconnect().await;
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_launch_split_application(move |index| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                let Some(application) = usize::try_from(index)
                    .ok()
                    .and_then(|index| window.get_split_applications().row_data(index))
                else {
                    window.set_detail("The selected application is no longer available".into());
                    return;
                };
                launch_split_application(
                    Arc::clone(&daemon),
                    Arc::clone(&runtime),
                    window.as_weak(),
                    application.path.to_string(),
                );
            });
    }
    {
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window.upgrade().unwrap().on_custom_list_edit(move |index| {
            let rows = usize::try_from(index).ok().and_then(|index| {
                custom_list_edit_rows(&relay_locations.lock().expect("relay location lock"), index)
            });
            if let (Some(window), Some(rows)) = (window.upgrade(), rows) {
                window.set_custom_list_edit_locations(ModelRc::new(VecModel::from(rows)));
            }
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window.upgrade().unwrap().on_custom_list_location_toggle(
            move |list_index, relay_index, selected| {
                let update = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    update_custom_list_location(
                        &mut state,
                        usize::try_from(list_index).ok(),
                        usize::try_from(relay_index).ok(),
                        selected,
                    )
                };
                let Some((list, rows)) = update else {
                    return;
                };
                if let Some(window) = window.upgrade() {
                    window.set_custom_list_edit_locations(ModelRc::new(VecModel::from(rows)));
                }
                let daemon = Arc::clone(&daemon);
                let relay_locations = Arc::clone(&relay_locations);
                let window = window.clone();
                runtime.spawn(async move {
                    match daemon
                        .update_custom_list(list.id, list.name, list.locations)
                        .await
                    {
                        Ok(()) => reload_location_models(daemon, relay_locations, window).await,
                        Err(error) => show_window_error(window, error),
                    }
                });
            },
        );
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_custom_list_create(move |name| {
                let daemon = Arc::clone(&daemon);
                let relay_locations = Arc::clone(&relay_locations);
                let window = window.clone();
                runtime.spawn(async move {
                    match daemon
                        .create_custom_list(name.to_string(), Vec::new())
                        .await
                    {
                        Ok(_) => reload_location_models(daemon, relay_locations, window).await,
                        Err(error) => show_window_error(window, error),
                    }
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_custom_list_rename(move |index, name| {
                let list = usize::try_from(index).ok().and_then(|index| {
                    let state = relay_locations.lock().expect("relay location lock");
                    let id = state.relays.get(index)?.custom_list_id.as_ref()?;
                    state
                        .custom_lists
                        .iter()
                        .find(|list| &list.id == id)
                        .cloned()
                });
                let daemon = Arc::clone(&daemon);
                let relay_locations = Arc::clone(&relay_locations);
                let window = window.clone();
                runtime.spawn(async move {
                    let result = match list {
                        Some(list) => {
                            daemon
                                .update_custom_list(list.id, name.to_string(), list.locations)
                                .await
                        }
                        None => Err("The custom list is no longer available".to_owned()),
                    };
                    match result {
                        Ok(()) => reload_location_models(daemon, relay_locations, window).await,
                        Err(error) => show_window_error(window, error),
                    }
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_custom_list_delete(move |index| {
                let id = usize::try_from(index).ok().and_then(|index| {
                    relay_locations
                        .lock()
                        .expect("relay location lock")
                        .relays
                        .get(index)?
                        .custom_list_id
                        .clone()
                });
                let daemon = Arc::clone(&daemon);
                let relay_locations = Arc::clone(&relay_locations);
                let window = window.clone();
                runtime.spawn(async move {
                    let result = match id {
                        Some(id) => daemon.delete_custom_list(id).await,
                        None => Err("The custom list is no longer available".to_owned()),
                    };
                    match result {
                        Ok(()) => reload_location_models(daemon, relay_locations, window).await,
                        Err(error) => show_window_error(window, error),
                    }
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_import_overrides_json(move |json| {
                import_override_json(
                    Arc::clone(&daemon),
                    Arc::clone(&runtime),
                    window.clone(),
                    json.to_string(),
                );
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_import_overrides_path(move |path| match fs::read_to_string(path.as_str()) {
                Ok(json) => import_override_json(
                    Arc::clone(&daemon),
                    Arc::clone(&runtime),
                    window.clone(),
                    json,
                ),
                Err(error) => {
                    if let Some(window) = window.upgrade() {
                        window.set_override_import_error(
                            format!("Could not read settings file: {error}").into(),
                        );
                    }
                }
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_find_split_application(move |path| {
                if let Some(window) = window.upgrade() {
                    launch_split_application(
                        Arc::clone(&daemon),
                        Arc::clone(&runtime),
                        window.as_weak(),
                        path.to_string(),
                    );
                }
            });
    }
    {
        let window = window.as_weak();
        window.upgrade().unwrap().on_language_selected(move |code| {
            let language = language_name(code.as_str());
            if let Some(window) = window.upgrade() {
                if let Err(error) = desktop::set_language(code.as_str()) {
                    window.set_detail(error.into());
                    return;
                }
                window.set_language(language.into());
                let selected_code = code.to_string();
                window.set_languages(ModelRc::new(VecModel::from(language_rows(&selected_code))));
                window.set_settings_page(4);
            }
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_api_toggle(move |id, enabled| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                let result = daemon
                    .set_api_access_method_enabled(id.to_string(), enabled)
                    .await;
                let methods = daemon.api_access_methods().await.ok();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        if let Some(methods) = methods {
                            window.set_api_methods(api_method_rows(methods));
                        }
                        if let Err(error) = result {
                            window.set_detail(error.into());
                        }
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_api_use(move |id| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                let result = daemon.use_api_access_method(id.to_string()).await;
                let methods = daemon.api_access_methods().await.ok();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        if let Some(methods) = methods {
                            window.set_api_methods(api_method_rows(methods));
                        }
                        if let Err(error) = result {
                            window.set_detail(error.into());
                        }
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_api_test(move |id| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                let id = id.to_string();
                let result = daemon.test_api_access_method(id.clone()).await;
                let methods = daemon.api_access_methods().await.ok();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        if let Some(methods) = methods {
                            let rows = methods
                                .into_iter()
                                .map(|method| ApiAccessMethodData {
                                    test_result: if method.id == id {
                                        match result.as_ref() {
                                            Ok(true) => 1,
                                            Ok(false) | Err(_) => 0,
                                        }
                                    } else {
                                        -1
                                    },
                                    id: method.id.into(),
                                    name: method.name.into(),
                                    enabled: method.enabled,
                                    in_use: method.in_use,
                                    custom: method.custom,
                                    testing: false,
                                    proxy_type: method.proxy_type,
                                    server: method.server.into(),
                                    port: method.port.into(),
                                    username: method.username.into(),
                                    password: method.password.into(),
                                    cipher: method.cipher.into(),
                                })
                                .collect::<Vec<_>>();
                            window.set_api_methods(ModelRc::new(VecModel::from(rows)));
                        }
                        if let Err(error) = result {
                            window.set_detail(error.into());
                        }
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_api_delete(move |id| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                let result = daemon.remove_api_access_method(id.to_string()).await;
                let methods = daemon.api_access_methods().await.ok();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        if let Some(methods) = methods {
                            window.set_api_methods(api_method_rows(methods));
                        }
                        if let Err(error) = result {
                            window.set_detail(error.into());
                        }
                    }
                });
            });
        });
    }
    {
        let window = window.as_weak();
        window.upgrade().unwrap().on_api_edit(move |id| {
            if let Some(window) = window.upgrade() {
                let name = window
                    .get_api_methods()
                    .iter()
                    .find(|method| method.id == id)
                    .unwrap_or_default();
                window.set_api_editor_is_new(false);
                window.set_api_editor_id(id);
                window.set_api_editor_name(name.name);
                window.set_api_editor_type(name.proxy_type);
                window.set_api_editor_server(name.server);
                window.set_api_editor_port(name.port);
                window.set_api_editor_username(name.username);
                window.set_api_editor_password(name.password);
                window.set_api_editor_cipher(name.cipher);
            }
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_api_save(
            move |id, name, kind, server, port, username, password, cipher| {
                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                let port = port.as_str().parse::<u32>();
                runtime.spawn(async move {
                    let result = match port {
                        Ok(port) if port > 0 => {
                            daemon
                                .save_api_access_method(
                                    id.to_string(),
                                    name.to_string(),
                                    kind,
                                    server.to_string(),
                                    port,
                                    username.to_string(),
                                    password.to_string(),
                                    cipher.to_string(),
                                )
                                .await
                        }
                        _ => Err("API access method requires a valid port".to_owned()),
                    };
                    let methods = daemon.api_access_methods().await.ok();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            if let Some(methods) = methods {
                                window.set_api_methods(api_method_rows(methods));
                            }
                            match result {
                                Ok(()) => window.set_settings_page(8),
                                Err(error) => window.set_detail(error.into()),
                            }
                        }
                    });
                });
            },
        );
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_obfuscation_changed(move |method| {
                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                runtime.spawn(async move {
                    let mode = match method {
                        1 => ObfuscationMode::WireguardPort,
                        2 => ObfuscationMode::Lwo,
                        3 => ObfuscationMode::Quic,
                        4 => ObfuscationMode::Shadowsocks,
                        5 => ObfuscationMode::UdpOverTcp,
                        6 => ObfuscationMode::Off,
                        _ => ObfuscationMode::Auto,
                    };
                    if let Err(error) = daemon.set_obfuscation(mode).await {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                window.set_detail(error.into());
                            }
                        });
                    }
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_clear_overrides(move || {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                let result = daemon.clear_relay_overrides().await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        match result {
                            Ok(()) => apply_relay_overrides(&window, Vec::new()),
                            Err(error) => window.set_detail(error.into()),
                        }
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_obfuscation_port_submitted(move |method, value| {
                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                let mode = match method {
                    1 => ObfuscationMode::WireguardPort,
                    2 => ObfuscationMode::Lwo,
                    4 => ObfuscationMode::Shadowsocks,
                    _ => ObfuscationMode::UdpOverTcp,
                };
                let port = if value.is_empty() {
                    Ok(None)
                } else {
                    value
                        .as_str()
                        .parse::<u32>()
                        .map(Some)
                        .map_err(|_| "Port must be a number between 1 and 65535".to_owned())
                };
                runtime.spawn(async move {
                    let result = match port {
                        Ok(port @ (None | Some(1..=65535))) => {
                            daemon.set_obfuscation_port(mode, port).await
                        }
                        Ok(Some(_)) => Err("Port must be between 1 and 65535".to_owned()),
                        Err(error) => Err(error),
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            match result {
                                Ok(()) => {
                                    let display = if value.is_empty() {
                                        "Automatic".into()
                                    } else {
                                        value
                                    };
                                    match method {
                                        1 => window.set_wireguard_port(display),
                                        2 => window.set_lwo_port(display),
                                        4 => window.set_shadowsocks_port(display),
                                        _ => window.set_udp_over_tcp_port(display),
                                    }
                                }
                                Err(error) => window.set_detail(error.into()),
                            }
                        }
                    });
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_ip_version_changed(move |mode| {
                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                runtime.spawn(async move {
                    let mode = match mode {
                        1 => IpVersionMode::Ipv4,
                        2 => IpVersionMode::Ipv6,
                        _ => IpVersionMode::Automatic,
                    };
                    if let Err(error) = daemon.set_ip_version(mode).await {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                window.set_detail(error.into());
                            }
                        });
                    }
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_mtu_submitted(move |value| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            let value = value.trim().parse::<u32>().ok();
            runtime.spawn(async move {
                let result = if value.is_none_or(|value| (1_280..=1_420).contains(&value)) {
                    daemon.set_mtu(value).await
                } else {
                    Err("WireGuard MTU must be between 1280 and 1420".to_owned())
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let (Some(window), Err(error)) = (window.upgrade(), result) {
                        window.set_detail(error.into());
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_dns_add(move |address| {
            let Some(strong_window) = window.upgrade() else {
                return;
            };
            let mut addresses = dns_model_values(&strong_window);
            let address = address.trim();
            if address.parse::<IpAddr>().is_err() || addresses.iter().any(|item| item == address) {
                strong_window.set_dns_address_invalid(true);
                return;
            }
            strong_window.set_dns_address_invalid(false);
            strong_window.set_new_dns_address("".into());
            strong_window.set_custom_dns(true);
            addresses.push(address.to_owned());
            apply_dns_addresses(&strong_window, &addresses);
            let daemon = Arc::clone(&daemon);
            runtime.spawn(async move {
                let _ = daemon.set_custom_dns(true, addresses).await;
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_dns_edit(move |index, address| {
                let Some(strong_window) = window.upgrade() else {
                    return;
                };
                let mut addresses = dns_model_values(&strong_window);
                let address = address.trim();
                let Some(index) = usize::try_from(index)
                    .ok()
                    .filter(|index| *index < addresses.len())
                else {
                    return;
                };
                if address.parse::<IpAddr>().is_err()
                    || addresses
                        .iter()
                        .enumerate()
                        .any(|(candidate, item)| candidate != index && item == address)
                {
                    strong_window.set_dns_address_invalid(true);
                    return;
                }
                strong_window.set_dns_address_invalid(false);
                addresses[index] = address.to_owned();
                apply_dns_addresses(&strong_window, &addresses);
                let daemon = Arc::clone(&daemon);
                runtime.spawn(async move {
                    let _ = daemon.set_custom_dns(true, addresses).await;
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_dns_remove(move |index| {
            let Some(strong_window) = window.upgrade() else {
                return;
            };
            let mut addresses = dns_model_values(&strong_window);
            if let Ok(index) = usize::try_from(index)
                && index < addresses.len()
            {
                addresses.remove(index);
            }
            apply_dns_addresses(&strong_window, &addresses);
            strong_window.set_custom_dns(!addresses.is_empty());
            let daemon = Arc::clone(&daemon);
            runtime.spawn(async move {
                let _ = daemon
                    .set_custom_dns(!addresses.is_empty(), addresses)
                    .await;
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_refresh_devices(move || {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            let Some(strong_window) = window.upgrade() else {
                return;
            };
            let account_number = account_digits(strong_window.get_account_number().as_str());
            let current_device = strong_window.get_current_device_id().to_string();
            strong_window.set_devices_loading(true);
            runtime.spawn(async move {
                let result = daemon.devices(account_number).await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        window.set_devices_loading(false);
                        match result {
                            Ok(devices) => {
                                let rows = devices
                                    .into_iter()
                                    .map(|device| AccountDevice {
                                        is_current: device.id == current_device,
                                        id: device.id.into(),
                                        name: format_device_name(&device.name).into(),
                                        created: device.created.into(),
                                        deleting: false,
                                    })
                                    .collect::<Vec<_>>();
                                window.set_account_devices(ModelRc::new(VecModel::from(rows)));
                                window.set_devices_error("".into());
                            }
                            Err(error) => window.set_devices_error(error.into()),
                        }
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_remove_device(move |device_id| {
                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                let Some(strong_window) = window.upgrade() else {
                    return;
                };
                let account_number = account_digits(strong_window.get_account_number().as_str());
                runtime.spawn(async move {
                    let result = daemon
                        .remove_device(account_number, device_id.to_string())
                        .await;
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            match result {
                                Ok(()) => window.invoke_refresh_devices(),
                                Err(error) => window.set_remove_device_error(error.into()),
                            }
                        }
                    });
                });
            });
    }
    {
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_dismiss_remove_device_error(move || {
                if let Some(window) = window.upgrade() {
                    window.set_remove_device_error("".into());
                }
            });
    }
    {
        let window = window.as_weak();
        window.upgrade().unwrap().on_cancel_login(move || {
            if let Some(window) = window.upgrade() {
                window.set_login_state("idle".into());
                window.set_login_error_message("".into());
                window.set_account_page("login".into());
            }
        });
    }
    {
        let window = window.as_weak();
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        window.upgrade().unwrap().on_location_clicked(move || {
            let window = window.clone();
            let daemon = Arc::clone(&daemon);
            let relay_locations = Arc::clone(&relay_locations);
            runtime.spawn(async move {
                match daemon.location_settings().await {
                    Ok(settings) => {
                        let (rows, providers, chips, recents_enabled, ownership, selected_count) = {
                            let mut state = relay_locations.lock().expect("relay location lock");
                            set_location_settings(&mut state, settings);
                            (
                                build_relay_rows(&state),
                                provider_filter_rows(&state),
                                location_filter_chips(&state),
                                state.recents_enabled,
                                state.ownership_filter,
                                state.selected_providers.len(),
                            )
                        };
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                set_relay_rows(&window, rows);
                                window.set_relay_providers(ModelRc::new(VecModel::from(providers)));
                                window.set_relay_filters(ModelRc::new(VecModel::from(chips)));
                                window.set_relay_recents_enabled(recents_enabled);
                                window.set_relay_ownership_filter(ownership);
                                window.set_relay_selected_provider_count(
                                    i32::try_from(selected_count).unwrap_or(i32::MAX),
                                );
                            }
                        });
                    }
                    Err(error) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                window.set_detail(error.into());
                                window.set_page("main".into());
                            }
                        });
                    }
                }
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_relay_recents_changed(move |enabled| {
                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                runtime.spawn(async move {
                    let result = daemon.set_enable_recents(enabled).await;
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            match result {
                                Ok(()) => window.set_relay_recents_enabled(enabled),
                                Err(error) => {
                                    window.set_relay_recents_enabled(!enabled);
                                    window.set_detail(error.into());
                                }
                            }
                        }
                    });
                });
            });
    }
    {
        let window = window.as_weak();
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        window.upgrade().unwrap().on_relay_selected(move |index| {
            let window = window.clone();
            let daemon = Arc::clone(&daemon);
            let relay = usize::try_from(index).ok().and_then(|index| {
                relay_locations
                    .lock()
                    .expect("relay location lock")
                    .relays
                    .get(index)
                    .cloned()
            });
            let selected_label = relay
                .as_ref()
                .map(|relay| relay.label.clone())
                .unwrap_or_default();
            runtime.spawn(async move {
                let result = if let Some(relay) = relay {
                    daemon.select_relay(relay).await
                } else {
                    Err("The selected relay is no longer available".to_owned())
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        match result {
                            Ok(()) => {
                                window.set_location(selected_label.into());
                                window.set_page("main".into());
                            }
                            Err(error) => window.set_detail(error.into()),
                        }
                    }
                });
            });
        });
    }
    {
        let window = window.as_weak();
        let relay_locations = Arc::clone(&relay_locations);
        window.upgrade().unwrap().on_relay_toggled(move |index| {
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            let rows = {
                let mut state = relay_locations.lock().expect("relay location lock");
                if !state.expanded.insert(index) {
                    state.expanded.remove(&index);
                }
                build_relay_rows(&state)
            };
            if let Some(window) = window.upgrade() {
                set_relay_rows(&window, rows);
            }
        });
    }
    {
        let window = window.as_weak();
        let relay_locations = Arc::clone(&relay_locations);
        window
            .upgrade()
            .unwrap()
            .on_relay_provider_toggled(move |provider, selected| {
                let (rows, providers, chips, selected_count) = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    if selected {
                        state.selected_providers.insert(provider.to_string());
                    } else {
                        state.selected_providers.remove(provider.as_str());
                    }
                    (
                        build_relay_rows(&state),
                        provider_filter_rows(&state),
                        location_filter_chips(&state),
                        state.selected_providers.len(),
                    )
                };
                if let Some(window) = window.upgrade() {
                    set_relay_rows(&window, rows);
                    window.set_relay_providers(ModelRc::new(VecModel::from(providers)));
                    window.set_relay_selected_provider_count(
                        i32::try_from(selected_count).unwrap_or(i32::MAX),
                    );
                    window.set_relay_filters(ModelRc::new(VecModel::from(chips)));
                }
            });
    }
    {
        let window = window.as_weak();
        let relay_locations = Arc::clone(&relay_locations);
        window
            .upgrade()
            .unwrap()
            .on_relay_providers_select_all(move |selected| {
                let (rows, providers, chips, selected_count) = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    state.selected_providers = if selected {
                        state.providers.iter().cloned().collect()
                    } else {
                        HashSet::new()
                    };
                    (
                        build_relay_rows(&state),
                        provider_filter_rows(&state),
                        location_filter_chips(&state),
                        state.selected_providers.len(),
                    )
                };
                if let Some(window) = window.upgrade() {
                    set_relay_rows(&window, rows);
                    window.set_relay_providers(ModelRc::new(VecModel::from(providers)));
                    window.set_relay_filters(ModelRc::new(VecModel::from(chips)));
                    window.set_relay_selected_provider_count(
                        i32::try_from(selected_count).unwrap_or(i32::MAX),
                    );
                }
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let relay_locations = Arc::clone(&relay_locations);
        let window = window.as_weak();
        window.upgrade().unwrap().on_relay_filters_applied(move || {
            let (role, ownership, providers) = {
                let state = relay_locations.lock().expect("relay location lock");
                let role = if state.location_type == 0 {
                    mullvad_gui_slint::model::RelayRole::Entry
                } else {
                    mullvad_gui_slint::model::RelayRole::Exit
                };
                let ownership = match state.ownership_filter {
                    0 => OwnershipFilter::MullvadOwned,
                    1 => OwnershipFilter::Rented,
                    _ => OwnershipFilter::Any,
                };
                let providers = state.selected_providers.iter().cloned().collect();
                (role, ownership, providers)
            };
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                if let Err(error) = daemon.set_relay_filters(role, ownership, providers).await {
                    show_window_error(window, error);
                }
            });
        });
    }
    {
        let window = window.as_weak();
        let relay_locations = Arc::clone(&relay_locations);
        window
            .upgrade()
            .unwrap()
            .on_relay_filter_removed(move |id| {
                let (rows, providers, chips, ownership, selected_count) = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    match id.as_str() {
                        "providers" => {
                            state.selected_providers = state.providers.iter().cloned().collect();
                        }
                        "ownership" => state.ownership_filter = -1,
                        _ => {}
                    }
                    (
                        build_relay_rows(&state),
                        provider_filter_rows(&state),
                        location_filter_chips(&state),
                        state.ownership_filter,
                        state.selected_providers.len(),
                    )
                };
                if let Some(window) = window.upgrade() {
                    set_relay_rows(&window, rows);
                    window.set_relay_providers(ModelRc::new(VecModel::from(providers)));
                    window.set_relay_filters(ModelRc::new(VecModel::from(chips)));
                    window.set_relay_ownership_filter(ownership);
                    window.set_relay_selected_provider_count(
                        i32::try_from(selected_count).unwrap_or(i32::MAX),
                    );
                }
            });
    }
    {
        let window = window.as_weak();
        let relay_locations = Arc::clone(&relay_locations);
        window
            .upgrade()
            .unwrap()
            .on_relay_ownership_changed(move |ownership| {
                let (rows, chips) = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    state.ownership_filter = ownership;
                    (build_relay_rows(&state), location_filter_chips(&state))
                };
                if let Some(window) = window.upgrade() {
                    set_relay_rows(&window, rows);
                    window.set_relay_filters(ModelRc::new(VecModel::from(chips)));
                }
            });
    }
    {
        let window = window.as_weak();
        let relay_locations = Arc::clone(&relay_locations);
        window
            .upgrade()
            .unwrap()
            .on_relay_search_changed(move |query| {
                let rows = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    state.query = query.to_string();
                    build_relay_rows(&state)
                };
                if let Some(window) = window.upgrade() {
                    set_relay_rows(&window, rows);
                }
            });
    }
    {
        let window = window.as_weak();
        let relay_locations = Arc::clone(&relay_locations);
        window
            .upgrade()
            .unwrap()
            .on_relay_role_changed(move |role| {
                let rows = {
                    let mut state = relay_locations.lock().expect("relay location lock");
                    state.location_type = role;
                    build_relay_rows(&state)
                };
                if let Some(window) = window.upgrade() {
                    set_relay_rows(&window, rows);
                }
            });
    }
    window.on_account_clicked(|| {});
    window.on_settings_clicked(|| {});
    {
        let window = window.as_weak();
        window.upgrade().unwrap().on_settings_navigate(move |page| {
            if let Some(window) = window.upgrade() {
                window.set_settings_page(page);
            }
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let tray_status_sender = tray_status_sender.clone();
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_setting_changed(move |name, enabled| {
                let name = name.to_string();
                let desktop_result = match name.as_str() {
                    "auto-start" => Some(desktop::set_autostart(enabled)),
                    "notifications" => Some(desktop::set_notifications_enabled(enabled)),
                    "monochromatic-icon" => {
                        let result = desktop::set_monochromatic_enabled(enabled);
                        if result.is_ok() {
                            let _ = tray_status_sender.try_send(TrayUpdate::Monochromatic(enabled));
                        }
                        Some(result)
                    }
                    "animate-map" => Some(desktop::set_animate_map_enabled(enabled)),
                    _ => None,
                };
                if let Some(Err(error)) = desktop_result {
                    if let Some(window) = window.upgrade() {
                        window.set_detail(error.into());
                    }
                    return;
                }
                if matches!(
                    name.as_str(),
                    "auto-start" | "notifications" | "monochromatic-icon" | "animate-map"
                ) {
                    return;
                }

                if name == "beta-releases" {
                    refresh_release_info(
                        window.clone(),
                        Arc::clone(&runtime),
                        enabled || is_beta_version(env!("CARGO_PKG_VERSION")),
                    );
                }

                let daemon = Arc::clone(&daemon);
                let window = window.clone();
                let dns_addresses = window
                    .upgrade()
                    .map(|window| dns_model_values(&window))
                    .unwrap_or_default();
                if name == "custom-dns" && enabled && dns_addresses.is_empty() {
                    if let Some(window) = window.upgrade() {
                        window.set_custom_dns(false);
                        window.set_settings_page(14);
                    }
                    return;
                }
                runtime.spawn(async move {
                    let result = match name.as_str() {
                        "auto-connect" => {
                            daemon
                                .set_boolean_setting(BooleanSetting::AutoConnect, enabled)
                                .await
                        }
                        "allow-lan" => {
                            daemon
                                .set_boolean_setting(BooleanSetting::AllowLan, enabled)
                                .await
                        }
                        "ipv6" => {
                            daemon
                                .set_boolean_setting(BooleanSetting::EnableIpv6, enabled)
                                .await
                        }
                        "lockdown" => {
                            daemon
                                .set_boolean_setting(BooleanSetting::LockdownMode, enabled)
                                .await
                        }
                        "beta-releases" => {
                            daemon
                                .set_boolean_setting(BooleanSetting::ShowBetaReleases, enabled)
                                .await
                        }
                        "quantum-resistant" => daemon.set_quantum_resistant(enabled).await,
                        "block-ads" => daemon.set_dns_blocker(DnsBlocker::Ads, enabled).await,
                        "block-trackers" => {
                            daemon.set_dns_blocker(DnsBlocker::Trackers, enabled).await
                        }
                        "block-malware" => {
                            daemon.set_dns_blocker(DnsBlocker::Malware, enabled).await
                        }
                        "block-gambling" => {
                            daemon.set_dns_blocker(DnsBlocker::Gambling, enabled).await
                        }
                        "block-adult-content" => {
                            daemon
                                .set_dns_blocker(DnsBlocker::AdultContent, enabled)
                                .await
                        }
                        "block-social-media" => {
                            daemon
                                .set_dns_blocker(DnsBlocker::SocialMedia, enabled)
                                .await
                        }
                        "custom-dns" if !enabled => {
                            daemon.set_custom_dns(false, dns_addresses).await
                        }
                        "custom-dns" if dns_addresses.is_empty() => {
                            Err("Add at least one DNS server before enabling custom DNS".to_owned())
                        }
                        "custom-dns" => daemon.set_custom_dns(true, dns_addresses).await,
                        _ => Ok(()),
                    };
                    if let Err(error) = result {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                window.set_detail(error.into());
                            }
                        });
                    }
                });
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_daita_changed(move |enabled| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                if let Err(error) = daemon.set_daita(enabled).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            window.set_detail(error.into());
                        }
                    });
                }
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_multihop_changed(move |mode| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                let mode = match mode {
                    1 => MultihopMode::Always,
                    2 => MultihopMode::Never,
                    _ => MultihopMode::Auto,
                };
                if let Err(error) = daemon.set_multihop(mode).await {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            window.set_detail(error.into());
                        }
                    });
                }
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        window.on_quit_clicked(move || {
            let daemon = Arc::clone(&daemon);
            runtime.spawn(async move {
                let _ = daemon.disconnect().await;
                let _ = slint::invoke_from_event_loop(|| {
                    let _ = slint::quit_event_loop();
                });
            });
        });
    }
    {
        let window = window.as_weak();
        window.upgrade().unwrap().on_open_url(move |url| {
            if let Err(error) = webbrowser::open(url.as_str())
                && let Some(window) = window.upgrade()
            {
                window.set_detail(format!("Could not open link: {error}").into());
            }
        });
    }
    {
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_problem_report_send(move |email, message| {
                let Some(strong_window) = window.upgrade() else {
                    return;
                };
                strong_window.set_problem_report_state("sending".into());
                strong_window.set_problem_report_error("".into());
                let redact = account_digits(strong_window.get_account_number().as_str());
                let window = window.clone();
                let email = email.to_string();
                let message = message.to_string();
                runtime.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        collect_and_send_problem_report(email, message, redact)
                    })
                    .await
                    .map_err(|error| format!("Problem report task failed: {error}"))
                    .and_then(|result| result);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            match result {
                                Ok(()) => window.set_problem_report_state("sent".into()),
                                Err(error) => {
                                    window.set_problem_report_state("failed".into());
                                    window.set_problem_report_error(error.into());
                                }
                            }
                        }
                    });
                });
            });
    }
    window.on_copy_account_number(|value| {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(value.to_string());
        }
    });
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_buy_more_credit(move || {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                match daemon.www_auth_token().await {
                    Ok(token) => {
                        let url = format!("https://mullvad.net/account/?token={token}");
                        let _ = webbrowser::open(&url);
                    }
                    Err(error) => {
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                window.set_detail(error.into());
                            }
                        });
                    }
                }
            });
        });
    }
    {
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_login_account_number_edited(move |value| {
                if let Some(window) = window.upgrade() {
                    window
                        .set_login_account_number_valid(account_digits(value.as_str()).len() == 16);
                    window.set_login_error_message("".into());
                }
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_login(move |value| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            let account_number = account_digits(value.as_str());
            let attempted_account_number = account_number.clone();
            if let Some(window) = window.upgrade() {
                window.set_login_state("submitting".into());
                window.set_login_method("existing".into());
            }
            runtime.spawn(async move {
                let result = daemon.login(account_number).await;
                let account = if result.is_ok() {
                    daemon.account_status().await.ok()
                } else {
                    None
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        match result {
                            Ok(()) => {
                                if let Some(account) = account {
                                    apply_account(&window, account);
                                }
                                window.set_login_state("success".into());
                                if window.get_account_state().as_str() == "active" {
                                    window.set_page("main".into());
                                }
                            }
                            Err(error) => {
                                let lower = error.to_lowercase();
                                if lower.contains("too many devices")
                                    || lower.contains("maximum number of devices")
                                {
                                    window.set_login_state("too-many-devices".into());
                                    window.set_account_number(
                                        format_account_number(&attempted_account_number).into(),
                                    );
                                    window.set_account_page("devices".into());
                                    window.invoke_refresh_devices();
                                } else {
                                    window.set_login_state("error".into());
                                    window.set_login_error_message(error.into());
                                }
                            }
                        }
                    }
                });
            });
        });
    }
    {
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_select_saved_account(move |value| {
                if let Some(window) = window.upgrade() {
                    window.set_login_account_number(value.clone());
                    window.set_login_account_number_valid(true);
                    window.invoke_login(value);
                }
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_clear_account_history(move || {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            runtime.spawn(async move {
                let result = daemon.clear_account_history().await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        match result {
                            Ok(()) => window.set_saved_account_number("".into()),
                            Err(error) => window.set_login_error_message(error.into()),
                        }
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_create_account(move || {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            if let Some(window) = window.upgrade() {
                window.set_login_state("submitting".into());
                window.set_login_method("create".into());
            }
            runtime.spawn(async move {
                let result = daemon.create_account().await;
                let result = match result {
                    Ok(account_number) => daemon.login(account_number).await,
                    Err(error) => Err(error),
                };
                let account = if result.is_ok() {
                    daemon.account_status().await.ok()
                } else {
                    None
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        match result {
                            Ok(()) => {
                                if let Some(account) = account {
                                    apply_account(&window, account);
                                }
                                window.set_login_state("success".into());
                                if window.get_account_state().as_str() == "active" {
                                    window.set_page("main".into());
                                }
                            }
                            Err(error) => {
                                window.set_login_state("error".into());
                                window.set_login_error_message(error.into());
                            }
                        }
                    }
                });
            });
        });
    }
    {
        let window = window.as_weak();
        window
            .upgrade()
            .unwrap()
            .on_voucher_code_edited(move |value| {
                let voucher = voucher_code(value.as_str());
                if let Some(window) = window.upgrade() {
                    window.set_voucher_state("idle".into());
                    window.set_voucher_code_valid(voucher.len() >= 16);
                    window.set_voucher_code_is_account_number(
                        voucher.len() == 16 && voucher.bytes().all(|byte| byte.is_ascii_digit()),
                    );
                }
            });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        window.upgrade().unwrap().on_redeem_voucher(move |code| {
            let daemon = Arc::clone(&daemon);
            let window = window.clone();
            if let Some(window) = window.upgrade() {
                window.set_voucher_state("submitting".into());
            }
            runtime.spawn(async move {
                let result = daemon.submit_voucher(voucher_code(code.as_str())).await;
                let account = if result.is_ok() {
                    daemon.account_status().await.ok()
                } else {
                    None
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        match result {
                            Ok(detail) => {
                                if let Some(account) = account {
                                    apply_account(&window, account);
                                    window.set_page("account".into());
                                }
                                window.set_voucher_state("success".into());
                                window.set_voucher_result_detail(detail.into());
                            }
                            Err(error) => {
                                let state = if error.to_lowercase().contains("used") {
                                    "already_used"
                                } else if error.to_lowercase().contains("invalid") {
                                    "invalid"
                                } else {
                                    "error"
                                };
                                window.set_voucher_state(state.into());
                                window.set_voucher_result_detail(error.into());
                            }
                        }
                    }
                });
            });
        });
    }
    {
        let window = window.as_weak();
        window.upgrade().unwrap().on_dismiss_voucher(move || {
            if let Some(window) = window.upgrade() {
                window.set_voucher_state("idle".into());
                window.set_voucher_result_detail("".into());
                window.set_page(
                    if window.get_account_state().as_str() == "active" {
                        "main"
                    } else {
                        "expired"
                    }
                    .into(),
                );
            }
        });
    }
    {
        let window = window.as_weak();
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        window.upgrade().unwrap().on_logout_clicked(move || {
            let window = window.clone();
            let daemon = Arc::clone(&daemon);
            runtime.spawn(async move {
                let result = daemon.logout().await;
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(window) = window.upgrade() {
                        match result {
                            Ok(()) => {
                                apply_account(&window, AccountStatus::LoggedOut);
                            }
                            Err(error) => window.set_detail(error.into()),
                        }
                    }
                });
            });
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let tray_status_sender = tray_status_sender.clone();
        let window = window.as_weak();
        let map_animator = Arc::clone(&map_animator);
        runtime.spawn(async move {
            loop {
                if let Ok(status) = daemon.tunnel_status().await {
                    let _ = tray_status_sender.try_send(TrayUpdate::Status(status.clone()));
                    let window = window.clone();
                    let map_animator = Arc::clone(&map_animator);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            apply_status(&window, &status, &map_animator);
                        }
                    });
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let window = window.as_weak();
        runtime.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60 * 60)).await;
                if let Ok(status) = daemon.account_status().await {
                    let window = window.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(window) = window.upgrade() {
                            apply_account(&window, status);
                        }
                    });
                }
            }
        });
    }
    {
        let daemon = Arc::clone(&daemon);
        let runtime = Arc::clone(&runtime);
        let window = window.as_weak();
        runtime.spawn(async move {
            while let Ok(command) = desktop_receiver.recv().await {
                match command {
                    DesktopCommand::Connect => {
                        let _ = daemon.connect().await;
                    }
                    DesktopCommand::Reconnect => {
                        let _ = daemon.reconnect().await;
                    }
                    DesktopCommand::Disconnect => {
                        let _ = daemon.disconnect().await;
                    }
                    DesktopCommand::DisconnectAndQuit => {
                        let _ = daemon.disconnect().await;
                        let _ = slint::invoke_from_event_loop(|| {
                            let _ = slint::quit_event_loop();
                        });
                    }
                    DesktopCommand::Show => {
                        let window = window.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(window) = window.upgrade() {
                                let _ = window.show();
                            }
                        });
                    }
                }
            }
        });
    }
    refresh_status(&window, &daemon, &runtime, &map_animator);
    refresh_account(&window, &daemon, &runtime);
    refresh_settings(&window, &daemon, &runtime);
    refresh_release_info(
        window.as_weak(),
        Arc::clone(&runtime),
        window.get_beta_releases() || is_beta_version(env!("CARGO_PKG_VERSION")),
    );
    if initial_page == "locations"
        && let Ok(settings) = runtime.block_on(daemon.location_settings())
    {
        let (rows, providers, chips, recents_enabled, ownership, selected_count) = {
            let mut state = relay_locations.lock().expect("relay location lock");
            set_location_settings(&mut state, settings);
            (
                build_relay_rows(&state),
                provider_filter_rows(&state),
                location_filter_chips(&state),
                state.recents_enabled,
                state.ownership_filter,
                state.selected_providers.len(),
            )
        };
        set_relay_rows(&window, rows);
        window.set_relay_providers(ModelRc::new(VecModel::from(providers)));
        window.set_relay_filters(ModelRc::new(VecModel::from(chips)));
        window.set_relay_recents_enabled(recents_enabled);
        window.set_relay_ownership_filter(ownership);
        window.set_relay_selected_provider_count(i32::try_from(selected_count).unwrap_or(i32::MAX));
    }
    if window.get_account_state().as_str() == "expired" {
        window.set_page("expired".into());
    } else if window.get_account_state().as_str() == "revoked" {
        window.set_page("revoked".into());
    } else if initial_page != "main" || window.get_logged_in() {
        window.set_page(initial_page.into());
    }
    if !start_in_background {
        window.show()?;
    }
    let _runtime_guard = runtime.enter();
    slint::run_event_loop_until_quit()
}

fn is_beta_version(version: &str) -> bool {
    version
        .split(['.', '-', '+'])
        .any(|part| matches!(part.to_ascii_lowercase().as_str(), "alpha" | "beta" | "rc"))
}

fn refresh_release_info(
    window: slint::Weak<MullvadWindow>,
    runtime: Arc<tokio::runtime::Runtime>,
    include_prereleases: bool,
) {
    runtime.spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            latest_github_release(env!("CARGO_PKG_VERSION"), include_prereleases)
        })
        .await;
        let Ok(Ok(release)) = result else {
            return;
        };
        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            if let Some(release) = release {
                window.set_update_available(true);
                window.set_suggested_version(release.version.to_string().into());
                window.set_update_url(release.url.into());
            } else {
                window.set_update_available(false);
                window.set_suggested_version("".into());
                window.set_update_url(GITHUB_RELEASES_URL.into());
            }
        });
    });
}

fn latest_github_release(
    current_version: &str,
    include_prereleases: bool,
) -> Result<Option<ReleaseInfo>, String> {
    let mut response = ureq::get(GITHUB_RELEASES_API)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "mullvad-gui-slint")
        .call()
        .map_err(|error| format!("Could not query GitHub releases: {error}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("Could not read GitHub release response: {error}"))?;
    select_github_release(current_version, include_prereleases, &body)
}

fn select_github_release(
    current_version: &str,
    include_prereleases: bool,
    body: &str,
) -> Result<Option<ReleaseInfo>, String> {
    let current = semver::Version::parse(current_version.trim_start_matches('v'))
        .map_err(|error| format!("Invalid current app version: {error}"))?;
    let releases: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| format!("Could not parse GitHub release response: {error}"))?;
    let releases = releases
        .as_array()
        .ok_or_else(|| "GitHub release response was not a list".to_owned())?;

    let latest = releases
        .iter()
        .filter(|release| !release["draft"].as_bool().unwrap_or(true))
        .filter(|release| include_prereleases || !release["prerelease"].as_bool().unwrap_or(false))
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let version = semver::Version::parse(tag.trim_start_matches('v')).ok()?;
            let url = release["html_url"].as_str()?.to_owned();
            Some(ReleaseInfo { version, url })
        })
        .filter(|release| release.version > current)
        .max_by(|left, right| left.version.cmp(&right.version));
    Ok(latest)
}

fn collect_and_send_problem_report(
    email: String,
    message: String,
    redact: String,
) -> Result<(), String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let report_path = env::temp_dir().join(format!(
        "mullvad-gui-slint-{}-{timestamp}.log",
        process::id()
    ));
    let mut collect = Command::new("mullvad-problem-report");
    collect.arg("collect").arg("--output").arg(&report_path);
    if !redact.is_empty() {
        collect.arg("--redact").arg(redact);
    }
    run_problem_report_command(collect, "collect")?;

    let mut send = Command::new("mullvad-problem-report");
    send.arg("send")
        .arg("--email")
        .arg(email)
        .arg("--message")
        .arg(message)
        .arg("--report")
        .arg(&report_path);
    let result = run_problem_report_command(send, "send");
    let _ = fs::remove_file(report_path);
    result
}

fn run_problem_report_command(mut command: Command, operation: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("Could not {operation} problem report: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let detail = if stderr.is_empty() {
        output.status.to_string()
    } else {
        stderr
    };
    Err(format!("Could not {operation} problem report: {detail}"))
}

fn set_location_settings(state: &mut RelayUiState, settings: LocationSettings) {
    state.recents_enabled = settings.recents_enabled;
    state.selected_entry = settings.selected_entry;
    state.selected_exit = settings.selected_exit;
    state.custom_lists = settings.custom_lists.clone();
    let previous_providers = std::mem::take(&mut state.providers);
    state.providers = settings.providers;
    if previous_providers.is_empty() {
        state
            .selected_providers
            .extend(state.providers.iter().cloned());
    } else {
        state
            .selected_providers
            .retain(|provider| state.providers.contains(provider));
        for provider in &state.providers {
            if !previous_providers.contains(provider) {
                state.selected_providers.insert(provider.clone());
            }
        }
    }
    state.relays = settings.recents;
    state.recent_count = state.relays.len();
    for list in settings.custom_lists {
        state.relays.push(RelayLocation {
            label: list.name,
            country_code: String::new(),
            city_code: None,
            hostname: None,
            custom_list_id: Some(list.id),
            depth: 0,
            provider: None,
            owned: None,
            daita: false,
        });
        state
            .relays
            .extend(list.locations.into_iter().map(|mut relay| {
                relay.depth = relay.depth.saturating_add(1);
                relay
            }));
    }
    state.custom_count = state.relays.len() - state.recent_count;
    state.relays.extend(settings.relays);
    state.expanded.clear();
}

fn custom_list_edit_rows(
    state: &RelayUiState,
    list_source_index: usize,
) -> Option<Vec<CustomListLocationData>> {
    let list_id = state
        .relays
        .get(list_source_index)?
        .custom_list_id
        .as_ref()?;
    let list = state.custom_lists.iter().find(|list| &list.id == list_id)?;
    let all_start = state.recent_count + state.custom_count;
    Some(
        state.relays[all_start..]
            .iter()
            .enumerate()
            .map(|(offset, relay)| CustomListLocationData {
                label: relay_display_label(relay).into(),
                depth: i32::from(relay.depth),
                source_index: i32::try_from(all_start + offset).unwrap_or(i32::MAX),
                selected: list
                    .locations
                    .iter()
                    .any(|selected| relay_constraints_match(selected, relay)),
            })
            .collect(),
    )
}

fn custom_list_choice_rows(
    state: &RelayUiState,
    relay_source_index: usize,
) -> Vec<CustomListChoiceData> {
    let Some(relay) = state.relays.get(relay_source_index) else {
        return Vec::new();
    };
    state
        .custom_lists
        .iter()
        .filter_map(|list| {
            let list_source_index = state.relays.iter().position(|candidate| {
                candidate.depth == 0
                    && candidate.custom_list_id.as_deref() == Some(list.id.as_str())
            })?;
            Some(CustomListChoiceData {
                label: list.name.clone().into(),
                list_source_index: i32::try_from(list_source_index).unwrap_or(i32::MAX),
                selected: list
                    .locations
                    .iter()
                    .any(|selected| relay_constraints_match(selected, relay)),
            })
        })
        .collect()
}

fn update_custom_list_location(
    state: &mut RelayUiState,
    list_source_index: Option<usize>,
    relay_source_index: Option<usize>,
    selected: bool,
) -> Option<(CustomListSummary, Vec<CustomListLocationData>)> {
    let list_source_index = list_source_index?;
    let relay_source_index = relay_source_index?;
    let list_id = state
        .relays
        .get(list_source_index)?
        .custom_list_id
        .clone()?;
    let relay = state.relays.get(relay_source_index)?.clone();
    let list = state
        .custom_lists
        .iter_mut()
        .find(|list| list.id == list_id)?;
    if selected {
        if !list
            .locations
            .iter()
            .any(|existing| relay_constraints_match(existing, &relay))
        {
            list.locations.push(relay);
        }
    } else {
        list.locations
            .retain(|existing| !relay_constraints_match(existing, &relay));
    }
    let list = list.clone();
    let rows = custom_list_edit_rows(state, list_source_index)?;
    Some((list, rows))
}

/// Updating each changed row in place (rather than swapping in a brand new
/// model) keeps the `ListView`'s existing row items alive, so per-row
/// `animate` transitions (checkmark, expand/collapse) actually get a "from"
/// value to animate from, and the list's scroll offset isn't disturbed by
/// tearing down and recreating every row. A full reset is only needed the
/// first time, or when the row count itself changes (the custom-list
/// placeholder row).
fn set_relay_rows(window: &MullvadWindow, rows: Vec<LocationRowData>) {
    let existing = window.get_relay_rows();
    if let Some(model) = existing
        .as_any()
        .downcast_ref::<VecModel<LocationRowData>>()
        && model.row_count() == rows.len()
    {
        for (index, row) in rows.into_iter().enumerate() {
            if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        }
        return;
    }
    let viewport_y = window.get_relay_list_viewport_y();
    MullvadWindow::set_relay_rows(window, ModelRc::new(VecModel::from(rows)));
    window.set_relay_list_viewport_y(viewport_y);
}

fn build_relay_rows(state: &RelayUiState) -> Vec<LocationRowData> {
    let query = state.query.trim().to_lowercase();
    let all_start = state.recent_count + state.custom_count;
    let mut ancestors = Vec::<usize>::new();
    let mut visible = Vec::with_capacity(state.relays.len());

    for (index, relay) in state.relays.iter().enumerate() {
        if index == all_start {
            ancestors.clear();
        }
        if index >= state.recent_count {
            ancestors.truncate(usize::from(relay.depth));
        }

        let is_visible = if query.is_empty() {
            if index < state.recent_count {
                true
            } else {
                relay.depth == 0
                    || ancestors
                        .iter()
                        .all(|parent| state.expanded.contains(parent))
            }
        } else if index < state.recent_count {
            relay.label.to_lowercase().contains(&query)
        } else {
            let section_start = if index < all_start {
                state.recent_count
            } else {
                all_start
            };
            let section = if index < all_start {
                &state.relays[state.recent_count..all_start]
            } else {
                &state.relays[all_start..]
            };
            let section_index = index - section_start;
            relay_subtree_matches(section, section_index, &query)
                || relay_ancestors_match(section, section_index, &query)
        };
        visible.push(is_visible && relay_matches_filters(state, relay));
        if index >= state.recent_count {
            ancestors.push(index);
        }
    }

    let first_recent = visible[..state.recent_count]
        .iter()
        .position(|is_visible| *is_visible);
    let first_custom = visible[state.recent_count..all_start]
        .iter()
        .position(|is_visible| *is_visible)
        .map(|index| index + state.recent_count);
    let first_all = visible[all_start..]
        .iter()
        .position(|is_visible| *is_visible)
        .map(|index| index + all_start);
    let last_recent = visible[..state.recent_count]
        .iter()
        .rposition(|is_visible| *is_visible);
    let last_custom = visible[state.recent_count..all_start]
        .iter()
        .rposition(|is_visible| *is_visible)
        .map(|index| index + state.recent_count);
    let last_all = visible[all_start..]
        .iter()
        .rposition(|is_visible| *is_visible)
        .map(|index| index + all_start);
    let total_relay_count = state.relays[all_start..]
        .iter()
        .filter(|relay| relay.depth == 2)
        .count();
    let visible_relay_count = state.relays[all_start..]
        .iter()
        .enumerate()
        .filter(|(index, relay)| {
            relay.depth == 2
                && relay_matches_filters(state, relay)
                && (query.is_empty() || visible[all_start + index])
        })
        .count();
    let all_section_detail = if visible_relay_count != total_relay_count {
        format!("Showing {visible_relay_count} of {total_relay_count}")
    } else {
        String::new()
    };
    let mut rows = state
        .relays
        .iter()
        .enumerate()
        .map(|(index, relay)| {
            let section_end = if index < state.recent_count {
                state.recent_count
            } else if index < all_start {
                all_start
            } else {
                state.relays.len()
            };
            let next_visible = ((index + 1)..section_end).find(|next| visible[*next]);
            let expandable = index >= state.recent_count
                && state
                    .relays
                    .get(index + 1)
                    .is_some_and(|next| next.depth > relay.depth);
            LocationRowData {
                label: relay_display_label(relay).into(),
                depth: if index < state.recent_count {
                    0
                } else {
                    i32::from(relay.depth)
                },
                source_index: i32::try_from(index).unwrap_or(i32::MAX),
                expandable,
                expanded: state.expanded.contains(&index) || !query.is_empty(),
                selected: (if state.location_type == 0 {
                    state.selected_entry.as_ref()
                } else {
                    state.selected_exit.as_ref()
                })
                .is_some_and(|selected| relay_constraints_match(selected, relay)),
                disabled: false,
                visible: visible[index],
                has_menu: index >= state.recent_count
                    && index < all_start
                    && relay.depth == 0
                    && relay.custom_list_id.is_some(),
                last_in_section: last_recent == Some(index)
                    || last_custom == Some(index)
                    || last_all == Some(index),
                joined_above: index >= state.recent_count && relay.depth > 0,
                joined_below: index >= state.recent_count
                    && next_visible.is_some_and(|next| state.relays[next].depth > 0),
                section_start: first_recent == Some(index)
                    || first_custom == Some(index)
                    || first_all == Some(index),
                section_title: if first_recent == Some(index) {
                    "Recents".into()
                } else if first_custom == Some(index) {
                    "Custom lists".into()
                } else if first_all == Some(index) {
                    "All locations".into()
                } else {
                    "".into()
                },
                section_detail: if first_all == Some(index) {
                    all_section_detail.clone().into()
                } else {
                    "".into()
                },
                subtitle: if index < state.recent_count {
                    relay_breadcrumb(relay).into()
                } else if index >= state.recent_count
                    && index < all_start
                    && relay.depth == 0
                    && !expandable
                {
                    "Empty".into()
                } else {
                    "".into()
                },
                placeholder: false,
                section_end: last_recent == Some(index) || last_custom == Some(index),
                can_add_to_list: !state.custom_lists.is_empty()
                    && relay.custom_list_id.is_none()
                    && (index < state.recent_count || index >= all_start),
                location_type: match relay.depth {
                    0 => "Country",
                    1 => "City",
                    _ => "Relay",
                }
                .into(),
            }
        })
        .collect::<Vec<_>>();
    if state.custom_count == 0 && query.is_empty() {
        rows.insert(
            state.recent_count,
            LocationRowData {
                label: "Add a custom list by clicking the “+” icon".into(),
                depth: 0,
                source_index: -1,
                expandable: false,
                expanded: false,
                selected: false,
                disabled: true,
                visible: true,
                has_menu: false,
                last_in_section: true,
                joined_above: false,
                joined_below: false,
                section_start: true,
                section_title: "Custom lists".into(),
                section_detail: "".into(),
                subtitle: "".into(),
                placeholder: true,
                section_end: true,
                can_add_to_list: false,
                location_type: "".into(),
            },
        );
    }
    rows
}

fn relay_breadcrumb(relay: &RelayLocation) -> String {
    if relay.custom_list_id.is_some() || relay.depth == 0 {
        return String::new();
    }
    if relay.depth == 1 {
        return relay
            .label
            .split_once(", ")
            .map(|(_, country)| country.to_owned())
            .unwrap_or_default();
    }
    relay
        .label
        .split_once(" - ")
        .map(|(_, parents)| parents.to_owned())
        .unwrap_or_default()
}

fn relay_matches_filters(state: &RelayUiState, relay: &RelayLocation) -> bool {
    let provider_matches = relay
        .provider
        .as_ref()
        .is_none_or(|provider| state.selected_providers.contains(provider));
    let ownership_matches = match (state.ownership_filter, relay.owned) {
        (-1, _) | (_, None) => true,
        (0, Some(owned)) => owned,
        (1, Some(owned)) => !owned,
        _ => true,
    };
    provider_matches && ownership_matches
}

fn provider_filter_rows(state: &RelayUiState) -> Vec<ProviderFilterData> {
    state
        .providers
        .iter()
        .map(|provider| ProviderFilterData {
            name: provider.clone().into(),
            selected: state.selected_providers.contains(provider),
        })
        .collect()
}

fn location_filter_chips(state: &RelayUiState) -> Vec<LocationFilterChip> {
    let mut chips = Vec::new();
    if state.selected_providers.len() < state.providers.len() {
        chips.push(LocationFilterChip {
            id: "providers".into(),
            label: format!("Providers: {}", state.selected_providers.len()).into(),
        });
    }
    if state.ownership_filter != -1 {
        chips.push(LocationFilterChip {
            id: "ownership".into(),
            label: if state.ownership_filter == 0 {
                "Ownership: Mullvad owned".into()
            } else {
                "Ownership: Rented".into()
            },
        });
    }
    chips
}

fn account_digits(value: &str) -> String {
    value.chars().filter(char::is_ascii_digit).collect()
}

fn voucher_code(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_uppercase())
        .take(16)
        .collect()
}

fn format_optional_port(port: Option<u32>) -> String {
    port.map_or_else(|| "Automatic".to_owned(), |port| port.to_string())
}

async fn reload_location_models(
    daemon: Arc<MullvadDaemon>,
    relay_locations: Arc<Mutex<RelayUiState>>,
    window: slint::Weak<MullvadWindow>,
) {
    match daemon.location_settings().await {
        Ok(settings) => {
            let (rows, providers, chips, recents_enabled, ownership, selected_count) = {
                let mut state = relay_locations.lock().expect("relay location lock");
                set_location_settings(&mut state, settings);
                (
                    build_relay_rows(&state),
                    provider_filter_rows(&state),
                    location_filter_chips(&state),
                    state.recents_enabled,
                    state.ownership_filter,
                    state.selected_providers.len(),
                )
            };
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = window.upgrade() {
                    set_relay_rows(&window, rows);
                    window.set_relay_providers(ModelRc::new(VecModel::from(providers)));
                    window.set_relay_filters(ModelRc::new(VecModel::from(chips)));
                    window.set_relay_recents_enabled(recents_enabled);
                    window.set_relay_ownership_filter(ownership);
                    window.set_relay_selected_provider_count(
                        i32::try_from(selected_count).unwrap_or(i32::MAX),
                    );
                }
            });
        }
        Err(error) => show_window_error(window, error),
    }
}

fn show_window_error(window: slint::Weak<MullvadWindow>, error: String) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = window.upgrade() {
            window.set_detail(error.into());
        }
    });
}

fn launch_split_application(
    daemon: Arc<MullvadDaemon>,
    runtime: Arc<tokio::runtime::Runtime>,
    window: slint::Weak<MullvadWindow>,
    path: String,
) {
    runtime.spawn(async move {
        let result = Command::new(&path)
            .spawn()
            .map_err(|error| format!("Could not launch {path}: {error}"));
        let result = match result {
            Ok(child) => {
                daemon
                    .add_split_tunnel_process(i32::try_from(child.id()).unwrap_or(i32::MAX))
                    .await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = window.upgrade() {
                    window.set_detail(error.into());
                }
            });
        }
    });
}

fn import_override_json(
    daemon: Arc<MullvadDaemon>,
    runtime: Arc<tokio::runtime::Runtime>,
    window: slint::Weak<MullvadWindow>,
    json: String,
) {
    runtime.spawn(async move {
        let result = daemon.apply_json_settings(json).await;
        let settings = if result.is_ok() {
            daemon.advanced_settings().await.ok()
        } else {
            None
        };
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window.upgrade() {
                match result {
                    Ok(()) => {
                        if let Some(settings) = settings {
                            apply_relay_overrides(&window, settings.relay_overrides);
                        }
                        window.set_override_import_error("".into());
                        window.set_override_status("IMPORT SUCCESS".into());
                        window.set_settings_page(12);
                    }
                    Err(error) => window.set_override_import_error(error.into()),
                }
            }
        });
    });
}

fn apply_relay_overrides(
    window: &MullvadWindow,
    overrides: Vec<mullvad_gui_slint::model::RelayOverride>,
) {
    window.set_override_status(
        if overrides.is_empty() {
            "NO OVERRIDES IMPORTED"
        } else {
            "OVERRIDES ACTIVE"
        }
        .into(),
    );
    window.set_relay_overrides(ModelRc::new(VecModel::from(
        overrides
            .into_iter()
            .map(|relay| RelayOverrideData {
                hostname: relay.hostname.into(),
                ipv4: relay.ipv4.unwrap_or_default().into(),
                ipv6: relay.ipv6.unwrap_or_default().into(),
            })
            .collect::<Vec<_>>(),
    )));
}

fn discover_desktop_applications() -> Vec<SettingsApplication> {
    let mut directories = vec![PathBuf::from("/usr/share/applications")];
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        directories.push(PathBuf::from(data_home).join("applications"));
    } else if let Some(home) = env::var_os("HOME") {
        directories.push(PathBuf::from(home).join(".local/share/applications"));
    }
    if let Some(data_dirs) = env::var_os("XDG_DATA_DIRS") {
        directories.extend(env::split_paths(&data_dirs).map(|path| path.join("applications")));
    }

    let mut applications = directories
        .into_iter()
        .flat_map(|directory| {
            fs::read_dir(directory)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| parse_desktop_application(&entry.path()))
        })
        .collect::<Vec<_>>();
    applications.sort_by_key(|application| application.name.to_lowercase());
    applications.dedup_by(|left, right| left.path == right.path);
    applications
}

fn parse_desktop_application(path: &Path) -> Option<SettingsApplication> {
    let contents = fs::read_to_string(path).ok()?;
    let mut in_desktop_entry = false;
    let mut name = None;
    let mut executable = None;
    let mut icon_name = None;
    let mut visible = true;
    for line in contents.lines() {
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        match line.split_once('=') {
            Some(("Name", value)) => name = Some(value.to_owned()),
            Some(("Exec", value)) => {
                executable = shlex::split(value).and_then(|parts| parts.into_iter().next())
            }
            Some(("Icon", value)) => icon_name = Some(value.to_owned()),
            Some(("NoDisplay" | "Hidden", "true")) => visible = false,
            Some(("Type", value)) if value != "Application" => visible = false,
            _ => {}
        }
    }
    if !visible {
        return None;
    }
    Some(SettingsApplication {
        name: name?.into(),
        path: executable?.into(),
        icon: icon_name
            .and_then(|name| resolve_icon(&name))
            .unwrap_or_default(),
        warning: false,
    })
}

/// Resolves a `.desktop` file's `Icon=` value (either an absolute path or an
/// icon-theme name per the freedesktop icon theme spec) to a loadable image.
fn resolve_icon(name: &str) -> Option<slint::Image> {
    let direct = Path::new(name);
    if direct.is_absolute() {
        return slint::Image::load_from_path(direct).ok();
    }

    // On NixOS (and other non-FHS systems) `/usr/share/icons` doesn't exist at
    // all — icon themes live under whatever `$XDG_DATA_DIRS` points at (e.g.
    // `/run/current-system/sw/share`, various nix profile paths), same as
    // `discover_desktop_applications`'s handling of `.desktop` files.
    let mut icon_base_dirs = Vec::new();
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        icon_base_dirs.push(PathBuf::from(data_home).join("icons"));
    } else if let Some(home) = env::var_os("HOME") {
        icon_base_dirs.push(PathBuf::from(&home).join(".local/share/icons"));
        icon_base_dirs.push(PathBuf::from(&home).join(".icons"));
    }
    if let Some(data_dirs) = env::var_os("XDG_DATA_DIRS") {
        icon_base_dirs.extend(env::split_paths(&data_dirs).map(|path| path.join("icons")));
    }
    icon_base_dirs.push(PathBuf::from("/usr/local/share/icons"));
    icon_base_dirs.push(PathBuf::from("/usr/share/icons"));
    icon_base_dirs.push(PathBuf::from("/run/current-system/sw/share/icons"));

    // Prefer higher-resolution/scalable variants; freedesktop themes typically
    // nest icons under <theme>/<size>/apps/<name>.<ext>.
    const SIZE_DIRS: &[&str] = &[
        "scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32",
    ];
    const EXTENSIONS: &[&str] = &["svg", "png"];

    // Collect every theme directory across every base path once, then search
    // size-first (scalable/highest-res across ALL themes) before falling back
    // to a smaller size — otherwise whichever theme happens to sort first in
    // the filesystem wins even if it only has a tiny low-res icon while a
    // later theme has the real scalable/512px one.
    let theme_dirs: Vec<PathBuf> = icon_base_dirs
        .iter()
        .flat_map(|base| fs::read_dir(base).into_iter().flatten())
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();

    for size in SIZE_DIRS {
        for theme_path in &theme_dirs {
            for ext in EXTENSIONS {
                let candidate = theme_path
                    .join(size)
                    .join("apps")
                    .join(format!("{name}.{ext}"));
                if candidate.is_file()
                    && let Ok(image) = slint::Image::load_from_path(&candidate)
                {
                    return Some(image);
                }
            }
        }
    }

    // Fall back to flat, unthemed icon directories.
    for ext in EXTENSIONS {
        let candidate = PathBuf::from("/usr/share/pixmaps").join(format!("{name}.{ext}"));
        if candidate.is_file()
            && let Ok(image) = slint::Image::load_from_path(&candidate)
        {
            return Some(image);
        }
    }

    None
}

fn dns_model_values(window: &MullvadWindow) -> Vec<String> {
    window
        .get_dns_addresses()
        .iter()
        .map(|address| address.to_string())
        .collect()
}

fn apply_dns_addresses(window: &MullvadWindow, addresses: &[String]) {
    window.set_dns_addresses(ModelRc::new(VecModel::from(
        addresses
            .iter()
            .map(|address| address.clone().into())
            .collect::<Vec<slint::SharedString>>(),
    )));
}

fn api_method_rows(
    methods: Vec<mullvad_gui_slint::model::ApiAccessMethodSummary>,
) -> ModelRc<ApiAccessMethodData> {
    ModelRc::new(VecModel::from(
        methods
            .into_iter()
            .map(|method| ApiAccessMethodData {
                id: method.id.into(),
                name: method.name.into(),
                enabled: method.enabled,
                in_use: method.in_use,
                custom: method.custom,
                testing: false,
                test_result: -1,
                proxy_type: method.proxy_type,
                server: method.server.into(),
                port: method.port.into(),
                username: method.username.into(),
                password: method.password.into(),
                cipher: method.cipher.into(),
            })
            .collect::<Vec<_>>(),
    ))
}

fn relay_display_label(relay: &RelayLocation) -> String {
    match relay.depth {
        0 => relay.label.clone(),
        1 => relay
            .label
            .split(',')
            .next()
            .unwrap_or(&relay.label)
            .to_owned(),
        _ => relay
            .hostname
            .clone()
            .or_else(|| relay.label.split(" - ").next().map(str::to_owned))
            .unwrap_or_else(|| relay.label.clone()),
    }
}

fn relay_constraints_match(left: &RelayLocation, right: &RelayLocation) -> bool {
    match (&left.custom_list_id, &right.custom_list_id) {
        (Some(left), Some(right)) => left == right,
        (None, None) => {
            left.country_code == right.country_code
                && left.city_code == right.city_code
                && left.hostname == right.hostname
        }
        _ => false,
    }
}

fn relay_subtree_matches(relays: &[RelayLocation], index: usize, query: &str) -> bool {
    let depth = relays[index].depth;
    relays[index..]
        .iter()
        .enumerate()
        .take_while(|(offset, relay)| *offset == 0 || relay.depth > depth)
        .map(|(_, relay)| relay)
        .any(|relay| relay.label.to_lowercase().contains(query))
}

fn relay_ancestors_match(relays: &[RelayLocation], index: usize, query: &str) -> bool {
    let relay = &relays[index];
    relay.label.to_lowercase().contains(query)
        || relays[..index].iter().rev().any(|candidate| {
            candidate.depth < relay.depth
                && candidate.country_code == relay.country_code
                && (candidate.depth == 0 || candidate.city_code == relay.city_code)
                && candidate.label.to_lowercase().contains(query)
        })
}

fn apply_account(window: &MullvadWindow, status: AccountStatus) {
    match status {
        AccountStatus::LoggedIn {
            account_number,
            device_id,
            device_name,
            expiry,
        } => {
            let expired = expiry.as_ref().is_some_and(|expiry| expiry.expired);
            window.set_logged_in(true);
            window.set_account_state(if expired { "expired" } else { "active" }.into());
            if window.get_account_page().as_str() != "devices" {
                window.set_account_page("account".into());
            }
            window.set_device_name(format_device_name(&device_name).into());
            window.set_current_device_id(device_id.into());
            window.set_account_number(format_account_number(&account_number).into());
            window.set_paid_until(
                expiry
                    .as_ref()
                    .map(|expiry| expiry.paid_until.clone())
                    .unwrap_or_else(|| "Currently unavailable".to_owned())
                    .into(),
            );
            window.set_paid_until_expired(expiry.as_ref().is_some_and(|expiry| expiry.expired));
            let time_left = expiry
                .as_ref()
                .filter(|expiry| expiry.show_in_header)
                .map(|expiry| expiry.remaining.clone())
                .unwrap_or_default();
            window.set_time_left(time_left.into());
            if expired {
                window.set_page("expired".into());
            }
        }
        AccountStatus::Revoked => {
            window.set_logged_in(false);
            window.set_current_device_id("".into());
            window.set_account_state("revoked".into());
            window.set_account_page("login".into());
            window.set_page("revoked".into());
            window.set_time_left("".into());
            window.set_paid_until("Currently unavailable".into());
            window.set_paid_until_expired(false);
        }
        AccountStatus::LoggedOut => {
            window.set_logged_in(false);
            window.set_current_device_id("".into());
            window.set_account_state("logged-out".into());
            window.set_account_page("login".into());
            window.set_page("account".into());
            window.set_device_name("".into());
            window.set_time_left("".into());
            window.set_account_number("".into());
            window.set_paid_until("Currently unavailable".into());
            window.set_paid_until_expired(false);
        }
    }
}

fn format_account_number(account_number: &str) -> String {
    account_number
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_device_name(device_name: &str) -> String {
    device_name
        .split_whitespace()
        .map(|word| {
            let mut characters = word.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(characters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn apply_status(
    window: &MullvadWindow,
    status: &TunnelStatus,
    map_animator: &Arc<Mutex<map_render::MapAnimator>>,
) {
    let next_status = match status {
        TunnelStatus::Connected { .. } => "CONNECTED",
        TunnelStatus::Connecting { .. } => "CONNECTING...",
        TunnelStatus::Disconnected { .. } => "DISCONNECTED",
        TunnelStatus::Unavailable(_) | TunnelStatus::Error(_) => "SERVICE UNAVAILABLE",
        TunnelStatus::Disconnecting {
            reconnecting: true, ..
        } => "RECONNECTING...",
        TunnelStatus::Disconnecting {
            reconnecting: false,
        } => "DISCONNECTING...",
    };
    if window.get_status().as_str() != next_status {
        window.set_connection_panel_expanded(false);
    }
    window.set_connection_in_address("".into());
    window.set_connection_out_ipv4("".into());
    window.set_connection_out_ipv6("".into());
    window.set_connection_protocol("".into());
    window.set_connection_features(ModelRc::new(VecModel::from(
        Vec::<slint::SharedString>::new(),
    )));
    let zoom = if matches!(status, TunnelStatus::Connected { .. }) {
        map_render::CONNECTED_ZOOM
    } else {
        map_render::DISCONNECTED_ZOOM
    };
    let marker = match status {
        TunnelStatus::Connected {
            coordinates: Some(_),
            ..
        } => map_render::MarkerState::Secure,
        TunnelStatus::Disconnected {
            coordinates: Some(_),
            ..
        } => map_render::MarkerState::Unsecure,
        _ => map_render::MarkerState::None,
    };
    map_animator.lock().expect("map animator lock").set_target(
        status.coordinates(),
        zoom,
        marker,
        window.get_animate_map(),
    );
    match status {
        TunnelStatus::Connected {
            location, details, ..
        } => {
            window.set_connected(true);
            window.set_transitioning(false);
            window.set_status("CONNECTED".into());
            apply_connection_details(window, location, details.as_ref());
        }
        TunnelStatus::Connecting {
            location, details, ..
        } => {
            window.set_connected(false);
            window.set_transitioning(true);
            window.set_status("CONNECTING...".into());
            apply_connection_details(window, location, details.as_ref());
        }
        TunnelStatus::Disconnected { location, .. } => {
            window.set_connected(false);
            window.set_transitioning(false);
            window.set_status("DISCONNECTED".into());
            window.set_detail(location.clone().unwrap_or_default().into());
        }
        TunnelStatus::Unavailable(message) | TunnelStatus::Error(message) => {
            window.set_connected(false);
            window.set_transitioning(false);
            window.set_status("SERVICE UNAVAILABLE".into());
            window.set_detail(message.clone().into());
        }
        TunnelStatus::Disconnecting { reconnecting } => {
            window.set_connected(false);
            window.set_transitioning(true);
            window.set_status(if *reconnecting {
                "RECONNECTING...".into()
            } else {
                "DISCONNECTING...".into()
            });
            window.set_detail("".into());
            window.set_location("Automatic".into());
        }
    }
}

fn apply_connection_details(
    window: &MullvadWindow,
    location: &Option<String>,
    details: Option<&ConnectionDetails>,
) {
    window.set_detail(location.clone().unwrap_or_default().into());
    let Some(details) = details else {
        window.set_location("".into());
        return;
    };

    let hostname = match (&details.hostname, &details.entry_hostname) {
        (Some(exit), Some(entry)) => format!("{exit} via {entry}"),
        (Some(exit), None) => exit.clone(),
        _ => String::new(),
    };
    window.set_location(hostname.into());
    window.set_connection_in_address(details.in_address.clone().unwrap_or_default().into());
    window.set_connection_out_ipv4(details.out_ipv4.clone().unwrap_or_default().into());
    window.set_connection_out_ipv6(details.out_ipv6.clone().unwrap_or_default().into());
    window.set_connection_protocol(details.protocol.clone().unwrap_or_default().into());
    window.set_connection_features(ModelRc::new(VecModel::from(
        details
            .features
            .iter()
            .cloned()
            .map(Into::into)
            .collect::<Vec<_>>(),
    )));
}

fn refresh_status(
    window: &MullvadWindow,
    daemon: &MullvadDaemon,
    runtime: &tokio::runtime::Runtime,
    map_animator: &Arc<Mutex<map_render::MapAnimator>>,
) {
    match runtime.block_on(daemon.tunnel_status()) {
        Ok(status) => apply_status(window, &status, map_animator),
        Err(error) => window.set_detail(error.into()),
    }
}

fn refresh_account(
    window: &MullvadWindow,
    daemon: &MullvadDaemon,
    runtime: &tokio::runtime::Runtime,
) {
    if let Ok(status) = runtime.block_on(daemon.account_status()) {
        apply_account(window, status);
    }
    if let Ok(history) = runtime.block_on(daemon.account_history()) {
        window.set_saved_account_number(
            history
                .as_deref()
                .map(format_account_number)
                .unwrap_or_default()
                .into(),
        );
    }
}

fn refresh_settings(
    window: &MullvadWindow,
    daemon: &MullvadDaemon,
    runtime: &tokio::runtime::Runtime,
) {
    window.set_animate_map(desktop::animate_map_enabled());
    window.set_auto_start(desktop::autostart_enabled());
    window.set_notifications(desktop::notifications_enabled());
    window.set_monochromatic_icon(desktop::monochromatic_enabled());
    let selected_language = desktop::language();
    window.set_language(language_name(&selected_language).into());
    window.set_languages(ModelRc::new(VecModel::from(language_rows(
        &selected_language,
    ))));

    if let Ok(settings) = runtime.block_on(daemon.settings()) {
        window.set_auto_connect(settings.auto_connect);
        window.set_allow_lan(settings.allow_lan);
        window.set_ipv6(settings.enable_ipv6);
        window.set_lockdown(settings.lockdown_mode);
        window.set_beta_releases(settings.show_beta_releases);
    }
    if let Ok(methods) = runtime.block_on(daemon.api_access_methods()) {
        window.set_api_methods(api_method_rows(methods));
    }
    let mut applications = discover_desktop_applications();
    if let Ok(split_tunnel) = runtime.block_on(daemon.split_tunnel_state()) {
        for path in split_tunnel.applications {
            if !applications
                .iter()
                .any(|application| application.path.as_str() == path)
            {
                let name = Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&path)
                    .to_owned();
                applications.push(SettingsApplication {
                    name: name.into(),
                    path: path.into(),
                    icon: Default::default(),
                    warning: false,
                });
            }
        }
    }
    window.set_split_applications(ModelRc::new(VecModel::from(applications)));
    if let Ok(settings) = runtime.block_on(daemon.advanced_settings()) {
        window.set_daita_enabled(settings.daita);
        window.set_block_ads(settings.block_ads);
        window.set_block_trackers(settings.block_trackers);
        window.set_block_malware(settings.block_malware);
        window.set_block_gambling(settings.block_gambling);
        window.set_block_adult_content(settings.block_adult_content);
        window.set_block_social_media(settings.block_social_media);
        window.set_custom_dns(settings.custom_dns_enabled);
        apply_dns_addresses(window, &settings.custom_dns_addresses);
        window.set_quantum_resistant(settings.quantum_resistant);
        window.set_multihop_mode(match settings.multihop {
            MultihopMode::Auto => 0,
            MultihopMode::Always => 1,
            MultihopMode::Never => 2,
        });
        window.set_ip_version(
            match settings.ip_version {
                mullvad_gui_slint::model::IpVersionMode::Automatic => "Automatic",
                mullvad_gui_slint::model::IpVersionMode::Ipv4 => "IPv4",
                mullvad_gui_slint::model::IpVersionMode::Ipv6 => "IPv6",
            }
            .into(),
        );
        window.set_ip_version_mode(match settings.ip_version {
            IpVersionMode::Automatic => 0,
            IpVersionMode::Ipv4 => 1,
            IpVersionMode::Ipv6 => 2,
        });
        window.set_obfuscation_method(match settings.obfuscation {
            ObfuscationMode::Auto => 0,
            ObfuscationMode::WireguardPort => 1,
            ObfuscationMode::Lwo => 2,
            ObfuscationMode::Quic => 3,
            ObfuscationMode::Shadowsocks => 4,
            ObfuscationMode::UdpOverTcp => 5,
            ObfuscationMode::Off => 6,
        });
        window.set_wireguard_port(format_optional_port(settings.wireguard_port).into());
        window.set_lwo_port(format_optional_port(settings.lwo_port).into());
        window.set_shadowsocks_port(format_optional_port(settings.shadowsocks_port).into());
        window.set_udp_over_tcp_port(format_optional_port(settings.udp_over_tcp_port).into());
        window.set_mtu(
            settings
                .mtu
                .map(|mtu| mtu.to_string())
                .unwrap_or_else(|| "Default".to_owned())
                .into(),
        );
        window.set_mtu_input(
            settings
                .mtu
                .map(|mtu| mtu.to_string())
                .unwrap_or_default()
                .into(),
        );
        apply_relay_overrides(window, settings.relay_overrides);
    }
}

// Upstream's getPreferredLocaleList() always pins "System default" first
// (SYSTEM_PREFERRED_LOCALE_KEY = "system"), then the rest sorted by
// display name — not English-first.
fn language_rows(selected_code: &str) -> Vec<LanguageData> {
    let system_default = std::iter::once(("system", "System default"));
    system_default
        .chain([
            ("de", "Deutsch"),
            ("en", "English"),
            ("es", "Español"),
            ("fr", "Français"),
            ("it", "Italiano"),
            ("ja", "日本語"),
            ("ko", "한국어"),
            ("nl", "Nederlands"),
            ("pl", "Polski"),
            ("pt", "Português"),
            ("ru", "Русский"),
            ("sv", "Svenska"),
            ("tr", "Türkçe"),
            ("zh", "中文"),
        ])
        .map(|(code, name)| LanguageData {
            code: code.into(),
            name: name.into(),
            selected: code == selected_code,
        })
        .collect()
}

fn language_name(code: &str) -> &'static str {
    match code {
        "de" => "Deutsch",
        "en" => "English",
        "es" => "Español",
        "fr" => "Français",
        "it" => "Italiano",
        "ja" => "日本語",
        "ko" => "한국어",
        "nl" => "Nederlands",
        "pl" => "Polski",
        "pt" => "Português",
        "ru" => "Русский",
        "sv" => "Svenska",
        "tr" => "Türkçe",
        "zh" => "中文",
        _ => "System default",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(label: &str, depth: u8) -> RelayLocation {
        RelayLocation {
            label: label.to_owned(),
            country_code: "se".to_owned(),
            city_code: (depth > 0).then(|| "got".to_owned()),
            hostname: (depth > 1).then(|| "se-got-wg-001".to_owned()),
            custom_list_id: None,
            depth,
            provider: None,
            owned: None,
            daita: false,
        }
    }

    fn location_state() -> RelayUiState {
        let mut state = RelayUiState::default();
        set_location_settings(
            &mut state,
            LocationSettings {
                recents: vec![relay("Sweden", 0)],
                custom_lists: vec![CustomListSummary {
                    id: "favorites".to_owned(),
                    name: "Favorites".to_owned(),
                    locations: Vec::new(),
                }],
                relays: vec![
                    relay("Sweden", 0),
                    relay("Gothenburg, Sweden", 1),
                    relay("se-got-wg-001 - Gothenburg, Sweden", 2),
                ],
                ..Default::default()
            },
        );
        state
    }

    #[test]
    fn location_rows_keep_official_sections_and_collapsed_tree() {
        let state = location_state();
        let rows = build_relay_rows(&state);
        assert_eq!(rows[0].section_title.as_str(), "Recents");
        assert_eq!(rows[1].section_title.as_str(), "Custom lists");
        assert_eq!(rows[2].section_title.as_str(), "All locations");
        assert_eq!(rows[2].section_detail.as_str(), "");
        assert!(rows[0].visible && rows[1].visible && rows[2].visible);
        assert!(!rows[3].visible && !rows[4].visible);
    }

    #[test]
    fn location_rows_expand_by_stable_daemon_index_and_filter_ancestors() {
        let mut state = location_state();
        state.expanded.insert(2);
        let rows = build_relay_rows(&state);
        assert!(rows[3].visible);
        assert!(!rows[4].visible);
        assert!(rows[2].joined_below);
        assert!(rows[3].joined_above);
        assert!(!rows[3].joined_below);

        state.query = "wg-001".to_owned();
        let rows = build_relay_rows(&state);
        assert!(rows[2].visible && rows[3].visible && rows[4].visible);
        assert_eq!(rows[4].label.as_str(), "se-got-wg-001");
    }

    #[test]
    fn voucher_normalization_preserves_official_alphanumeric_format() {
        assert_eq!(voucher_code("abcd-1234-efgh-5678"), "ABCD1234EFGH5678");
        assert_eq!(
            voucher_code("abcd 1234 efgh 5678 extra"),
            "ABCD1234EFGH5678"
        );
        assert_eq!(account_digits("1234 5678 9012 3456"), "1234567890123456");
    }

    #[test]
    fn beta_version_detection_recognizes_pre_release_channels() {
        assert!(is_beta_version("0.1.0-alpha.1"));
        assert!(is_beta_version("0.1.0-beta.2"));
        assert!(is_beta_version("0.1.0-rc.1"));
        assert!(!is_beta_version("0.1.0"));
    }

    #[test]
    fn github_release_selection_respects_channel_and_semver_order() {
        let releases = r#"[
            {"tag_name":"v0.3.0-beta.1","html_url":"https://example/beta","draft":false,"prerelease":true},
            {"tag_name":"v0.2.0","html_url":"https://example/stable","draft":false,"prerelease":false},
            {"tag_name":"v9.0.0","html_url":"https://example/draft","draft":true,"prerelease":false}
        ]"#;

        let stable = select_github_release("0.1.0", false, releases)
            .unwrap()
            .unwrap();
        assert_eq!(stable.version, semver::Version::new(0, 2, 0));
        assert_eq!(stable.url, "https://example/stable");

        let beta = select_github_release("0.1.0", true, releases)
            .unwrap()
            .unwrap();
        assert_eq!(
            beta.version,
            semver::Version::parse("0.3.0-beta.1").unwrap()
        );
        assert_eq!(beta.url, "https://example/beta");
        assert!(
            select_github_release("0.3.0", true, releases)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn custom_lists_include_expandable_location_hierarchy() {
        let mut state = RelayUiState::default();
        set_location_settings(
            &mut state,
            LocationSettings {
                custom_lists: vec![CustomListSummary {
                    id: "favorites".to_owned(),
                    name: "Favorites".to_owned(),
                    locations: vec![relay("Sweden", 0), relay("Gothenburg, Sweden", 1)],
                }],
                ..Default::default()
            },
        );
        let rows = build_relay_rows(&state);
        assert_eq!(rows[0].label.as_str(), "Favorites");
        assert!(rows[0].expandable && rows[0].has_menu);
        assert!(!rows[1].visible);

        state.expanded.insert(0);
        let rows = build_relay_rows(&state);
        assert!(rows[1].visible);
        assert!(!rows[2].visible);
    }

    #[test]
    fn custom_list_editor_updates_daemon_location_constraints() {
        let mut state = location_state();
        let rows = custom_list_edit_rows(&state, 1).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| !row.selected));

        let (list, rows) = update_custom_list_location(&mut state, Some(1), Some(3), true).unwrap();
        assert_eq!(list.id, "favorites");
        assert_eq!(list.locations.len(), 1);
        assert_eq!(list.locations[0].city_code.as_deref(), Some("got"));
        assert!(rows[1].selected);

        let (list, rows) =
            update_custom_list_location(&mut state, Some(1), Some(3), false).unwrap();
        assert!(list.locations.is_empty());
        assert!(!rows[1].selected);
    }

    #[test]
    fn location_selection_and_filters_are_reflected_in_rows_and_chips() {
        let mut owned = relay("se-got-wg-001 - Gothenburg, Sweden", 2);
        owned.provider = Some("Mullvad".to_owned());
        owned.owned = Some(true);
        let mut rented = relay("se-got-wg-002 - Gothenburg, Sweden", 2);
        rented.hostname = Some("se-got-wg-002".to_owned());
        rented.provider = Some("Partner".to_owned());
        rented.owned = Some(false);
        let mut state = RelayUiState::default();
        set_location_settings(
            &mut state,
            LocationSettings {
                relays: vec![
                    relay("Sweden", 0),
                    relay("Gothenburg, Sweden", 1),
                    owned.clone(),
                    rented,
                ],
                providers: vec!["Mullvad".to_owned(), "Partner".to_owned()],
                selected_exit: Some(owned),
                ..Default::default()
            },
        );
        state.expanded.extend([0, 1]);
        state.selected_providers.remove("Partner");
        state.ownership_filter = 0;
        let rows = build_relay_rows(&state);
        let all_locations = rows
            .iter()
            .find(|row| row.section_title.as_str() == "All locations")
            .unwrap();
        assert_eq!(all_locations.section_detail.as_str(), "Showing 1 of 2");
        let selected = rows.iter().find(|row| row.source_index == 2).unwrap();
        let filtered = rows.iter().find(|row| row.source_index == 3).unwrap();
        assert!(selected.selected && selected.visible);
        assert!(!filtered.visible);
        let chips = location_filter_chips(&state);
        assert_eq!(chips.len(), 2);
        assert_eq!(chips[0].label.as_str(), "Providers: 1");
        assert_eq!(chips[1].label.as_str(), "Ownership: Mullvad owned");
    }

    #[test]
    fn location_rows_match_upstream_empty_custom_section_and_recent_breadcrumbs() {
        let mut state = RelayUiState::default();
        set_location_settings(
            &mut state,
            LocationSettings {
                recents: vec![relay("Gothenburg, Sweden", 1)],
                relays: vec![relay("Sweden", 0), relay("Gothenburg, Sweden", 1)],
                ..Default::default()
            },
        );

        let rows = build_relay_rows(&state);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[0].subtitle.as_str(), "Sweden");
        assert!(!rows[0].joined_above && !rows[0].joined_below);
        assert!(rows[0].section_end);
        assert_eq!(rows[1].section_title.as_str(), "Custom lists");
        assert!(rows[1].placeholder && rows[1].section_end);
        assert_eq!(
            rows[1].label.as_str(),
            "Add a custom list by clicking the “+” icon"
        );
        assert_eq!(rows[2].section_title.as_str(), "All locations");
    }
}
