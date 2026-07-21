use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

mod map;

use gtk::gdk_pixbuf::prelude::PixbufLoaderExt;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow};
use mullvad_gtk::{
    controller::{Controller, DaemonCommand, FeatureApi},
    daemon::MullvadDaemon,
    desktop::{self, DesktopCommand, TrayUpdate},
    model::{
        AccountStatus, AdvancedSettings, AppSettings, BooleanSetting, DeviceSummary, DnsBlocker,
        MultihopMode, ObfuscationMode, RelayLocation, TunnelStatus,
    },
};

const APP_ID: &str = "io.github.Greenstorm5417.MullvadGTK";
const STATUS_CLASSES: [&str; 3] = ["connected", "transitioning", "disconnected"];

#[derive(Clone, Copy)]
enum AdvancedToggle {
    Dns(DnsBlocker),
    QuantumResistance,
    Daita,
}

struct AccountWidgets {
    state: gtk::Label,
    detail: gtk::Label,
    entry: gtk::Entry,
    login: gtk::Button,
    create: gtk::Button,
    logout: gtk::Button,
    voucher_entry: gtk::Entry,
    redeem_voucher: gtk::Button,
    manage_devices: gtk::Button,
    error: gtk::Label,
    account_number: Rc<RefCell<Option<String>>>,
}

fn main() -> gtk::glib::ExitCode {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Tokio runtime should initialize"),
    );
    let application = Application::builder().application_id(APP_ID).build();
    let start_in_background = std::env::args().any(|argument| argument == "--background");

    application.connect_activate(move |application| {
        if let Some(window) = application.active_window() {
            window.present();
        } else {
            build_ui(application, Arc::clone(&runtime), start_in_background);
        }
    });
    application.run()
}

fn build_ui(
    application: &Application,
    runtime: Arc<tokio::runtime::Runtime>,
    start_in_background: bool,
) {
    install_styles();

    let daemon = Arc::new(MullvadDaemon::default());
    let controller = Arc::new(Controller::new(daemon.as_ref().clone()));
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .hexpand(true)
        .vexpand(true)
        .build();

    let logo = embedded_svg(include_bytes!("../assets/images/logo-icon.svg"), 36, 36);
    let brand = embedded_svg(include_bytes!("../assets/images/logo-text.svg"), 116, 20);
    let header_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header_content.append(&logo);
    header_content.append(&brand);

    let header_bar = gtk::HeaderBar::builder()
        .title_widget(&header_content)
        .show_title_buttons(true)
        .css_classes(["app-header"])
        .build();
    let home_button = icon_button("go-previous-symbolic", "Back");
    home_button.set_visible(false);
    let account_button = icon_button("avatar-default-symbolic", "Account");
    let settings_button = icon_button("preferences-system-symbolic", "Settings");
    header_bar.pack_start(&home_button);
    header_bar.pack_end(&settings_button);
    header_bar.pack_end(&account_button);
    stack.connect_visible_child_name_notify({
        let home_button = home_button.clone();
        let account_button = account_button.clone();
        let settings_button = settings_button.clone();
        move |stack| {
            let main_visible = stack.visible_child_name().as_deref() == Some("main");
            home_button.set_visible(!main_visible);
            account_button.set_visible(main_visible);
            settings_button.set_visible(main_visible);
        }
    });

    let status_label = gtk::Label::builder()
        .label("CONNECTING TO DAEMON")
        .css_classes(["status-title", "transitioning"])
        .wrap(true)
        .halign(gtk::Align::Start)
        .justify(gtk::Justification::Left)
        .build();
    let detail_label = gtk::Label::builder()
        .label("Reading tunnel state")
        .css_classes(["status-detail"])
        .wrap(true)
        .halign(gtk::Align::Start)
        .justify(gtk::Justification::Left)
        .build();
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(42)
        .height_request(42)
        .build();
    let action_button = gtk::Button::builder()
        .label("Please wait")
        .css_classes(["tunnel-action"])
        .sensitive(false)
        .build();
    let reconnect_button = icon_button("view-refresh-symbolic", "Reconnect");
    reconnect_button.add_css_class("reconnect-action");
    reconnect_button.set_visible(false);
    let location_button = gtk::Button::builder()
        .label("Select location")
        .css_classes(["secondary-action"])
        .build();

    let status_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .css_classes(["connection-panel"])
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::End)
        .margin_bottom(16)
        .margin_start(16)
        .margin_end(16)
        .build();
    status_content.append(&spinner);
    status_content.append(&status_label);
    status_content.append(&detail_label);
    status_content.append(&location_button);
    let tunnel_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    tunnel_actions.append(&action_button);
    tunnel_actions.append(&reconnect_button);
    status_content.append(&tunnel_actions);
    let map_view = map::MapView::new();
    let main_page = gtk::Overlay::new();
    main_page.set_child(Some(&map_view.widget));
    main_page.add_overlay(&status_content);

    let (allow_lan_row, allow_lan) = switch_row("Local network sharing");
    let (auto_connect_row, auto_connect) = switch_row("Auto-connect");
    let (ipv6_row, enable_ipv6) = switch_row("Enable IPv6");
    let (lockdown_row, lockdown_mode) = switch_row("Lockdown mode");
    let (beta_row, show_beta) = switch_row("Show beta releases");
    let (notifications_row, enable_notifications) = switch_row("System notifications");
    enable_notifications.set_active(desktop::notifications_enabled());
    let (monochromatic_row, monochromatic_icon) = switch_row("Monochromatic tray icon");
    monochromatic_icon.set_active(desktop::monochromatic_enabled());
    let (autostart_row, autostart) = switch_row("Launch at login");
    autostart.set_active(desktop::autostart_enabled());
    let settings_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["settings-list"])
        .build();
    for row in [
        &allow_lan_row,
        &auto_connect_row,
        &ipv6_row,
        &lockdown_row,
        &beta_row,
        &notifications_row,
        &monochromatic_row,
        &autostart_row,
    ] {
        settings_list.append(row);
    }
    let split_tunnel_button = gtk::Button::builder()
        .label("Split tunneling")
        .css_classes(["secondary-action"])
        .build();
    let support_button = gtk::Button::builder()
        .label("Support and report an issue")
        .css_classes(["secondary-action"])
        .build();
    let advanced_settings_button = gtk::Button::builder()
        .label("Advanced VPN settings")
        .css_classes(["secondary-action"])
        .build();
    let settings_error = error_label();
    let settings_page = page_content();
    settings_page.append(&page_title("VPN settings"));
    settings_page.append(&settings_list);
    settings_page.append(&advanced_settings_button);
    settings_page.append(&split_tunnel_button);
    settings_page.append(&support_button);
    settings_page.append(&settings_error);

    let support_description = gtk::Label::builder()
        .label("Report Mullvad-GTK interface and packaging problems on GitHub. For VPN service or account support, contact Mullvad directly.")
        .css_classes(["status-detail"])
        .wrap(true)
        .halign(gtk::Align::Start)
        .build();
    let report_issue_button = gtk::Button::with_label("Report a Mullvad-GTK issue");
    report_issue_button.add_css_class("suggested-action");
    let open_issues_button = gtk::Button::with_label("View open issues");
    let support_error = error_label();
    let support_page = page_content();
    support_page.append(&page_title("Mullvad-GTK support"));
    support_page.append(&support_description);
    support_page.append(&report_issue_button);
    support_page.append(&open_issues_button);
    support_page.append(&support_error);

    let account_state = gtk::Label::builder()
        .label("Loading account...")
        .css_classes(["section-title"])
        .wrap(true)
        .build();
    let account_detail = gtk::Label::builder()
        .css_classes(["status-detail"])
        .selectable(true)
        .wrap(true)
        .build();
    let account_entry = gtk::Entry::builder()
        .placeholder_text("Mullvad account number")
        .input_purpose(gtk::InputPurpose::Digits)
        .build();
    let login_button = gtk::Button::with_label("Log in");
    login_button.add_css_class("suggested-action");
    let create_account_button = gtk::Button::with_label("Create account");
    let voucher_entry = gtk::Entry::builder()
        .placeholder_text("Voucher code")
        .visible(false)
        .build();
    let redeem_voucher_button = gtk::Button::with_label("Redeem voucher");
    redeem_voucher_button.set_visible(false);
    let voucher_status = gtk::Label::builder()
        .css_classes(["status-detail"])
        .wrap(true)
        .build();
    let manage_devices_button = gtk::Button::with_label("Manage devices");
    manage_devices_button.set_visible(false);
    let logout_button = gtk::Button::with_label("Log out");
    logout_button.set_visible(false);
    logout_button.add_css_class("destructive-action");
    let account_error = error_label();
    let account_page = page_content();
    account_page.append(&page_title("Account"));
    account_page.append(&account_state);
    account_page.append(&account_detail);
    account_page.append(&account_entry);
    account_page.append(&login_button);
    account_page.append(&create_account_button);
    account_page.append(&voucher_entry);
    account_page.append(&redeem_voucher_button);
    account_page.append(&voucher_status);
    account_page.append(&manage_devices_button);
    account_page.append(&logout_button);
    account_page.append(&account_error);

    let relay_model = gtk::StringList::new(&[]);
    let relay_dropdown = gtk::DropDown::builder()
        .model(&relay_model)
        .enable_search(true)
        .build();
    let apply_relay_button = gtk::Button::with_label("Select relay");
    apply_relay_button.add_css_class("suggested-action");
    let relay_error = error_label();
    let locations_page = page_content();
    locations_page.append(&page_title("Select location"));
    locations_page.append(&relay_dropdown);
    locations_page.append(&apply_relay_button);
    locations_page.append(&relay_error);

    let process_id = gtk::SpinButton::with_range(1.0, f64::from(i32::MAX), 1.0);
    process_id.set_numeric(true);
    let add_process_button = gtk::Button::with_label("Exclude process");
    let remove_process_button = gtk::Button::with_label("Include process");
    let clear_processes_button = gtk::Button::with_label("Clear exclusions");
    clear_processes_button.add_css_class("destructive-action");
    let excluded_processes = gtk::Label::builder()
        .label("No excluded processes")
        .css_classes(["status-detail"])
        .selectable(true)
        .wrap(true)
        .build();
    let split_error = error_label();
    let split_page = page_content();
    split_page.append(&page_title("Split tunneling"));
    split_page.append(&excluded_processes);
    split_page.append(&process_id);
    split_page.append(&add_process_button);
    split_page.append(&remove_process_button);
    split_page.append(&clear_processes_button);
    split_page.append(&split_error);

    let (ads_row, block_ads) = switch_row("Block ads");
    let (trackers_row, block_trackers) = switch_row("Block trackers");
    let (malware_row, block_malware) = switch_row("Block malware");
    let (adult_row, block_adult) = switch_row("Block adult content");
    let (gambling_row, block_gambling) = switch_row("Block gambling");
    let (social_row, block_social) = switch_row("Block social media");
    let (quantum_row, quantum_resistant) = switch_row("Quantum resistance");
    let (daita_row, daita) = switch_row("DAITA");
    let advanced_list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["settings-list"])
        .build();
    for row in [
        &ads_row,
        &trackers_row,
        &malware_row,
        &adult_row,
        &gambling_row,
        &social_row,
        &quantum_row,
        &daita_row,
    ] {
        advanced_list.append(row);
    }
    let multihop = gtk::DropDown::from_strings(&["Automatic", "Always", "Never"]);
    let obfuscation = gtk::DropDown::from_strings(&[
        "Automatic",
        "Off",
        "WireGuard port",
        "UDP over TCP",
        "Shadowsocks",
        "QUIC",
        "LWO",
    ]);
    let mtu = gtk::SpinButton::with_range(0.0, 65_535.0, 1.0);
    mtu.set_tooltip_text(Some("Use 0 for the automatic MTU"));
    let apply_advanced = gtk::Button::with_label("Apply tunnel settings");
    apply_advanced.add_css_class("suggested-action");
    let advanced_error = error_label();
    let advanced_page = page_content();
    advanced_page.append(&page_title("Advanced VPN settings"));
    advanced_page.append(&advanced_list);
    advanced_page.append(&field_label("Multihop"));
    advanced_page.append(&multihop);
    advanced_page.append(&field_label("Obfuscation"));
    advanced_page.append(&obfuscation);
    advanced_page.append(&field_label("WireGuard MTU"));
    advanced_page.append(&mtu);
    advanced_page.append(&apply_advanced);
    advanced_page.append(&advanced_error);

    let devices_label = gtk::Label::builder()
        .label("Loading devices...")
        .css_classes(["status-detail"])
        .selectable(true)
        .wrap(true)
        .halign(gtk::Align::Start)
        .build();
    let device_id_entry = gtk::Entry::builder()
        .placeholder_text("Device ID to remove")
        .build();
    let remove_device_button = gtk::Button::with_label("Remove device");
    remove_device_button.add_css_class("destructive-action");
    let refresh_devices_button = gtk::Button::with_label("Refresh devices");
    let devices_error = error_label();
    let devices_page = page_content();
    devices_page.append(&page_title("Manage devices"));
    devices_page.append(&devices_label);
    devices_page.append(&device_id_entry);
    devices_page.append(&remove_device_button);
    devices_page.append(&refresh_devices_button);
    devices_page.append(&devices_error);

    stack.add_named(&main_page, Some("main"));
    stack.add_named(&scroll_page(&settings_page), Some("settings"));
    stack.add_named(&scroll_page(&account_page), Some("account"));
    stack.add_named(&scroll_page(&locations_page), Some("locations"));
    stack.add_named(&scroll_page(&split_page), Some("split-tunnel"));
    stack.add_named(&scroll_page(&advanced_page), Some("advanced"));
    stack.add_named(&scroll_page(&devices_page), Some("devices"));
    stack.add_named(&scroll_page(&support_page), Some("support"));
    stack.set_visible_child_name("main");

    let tray_available = Rc::new(Cell::new(false));
    let (desktop_command_sender, desktop_command_receiver) = async_channel::unbounded();
    let (tray_availability_sender, tray_availability_receiver) = async_channel::unbounded();
    let tray_status_sender =
        desktop::start_tray(&runtime, desktop_command_sender, tray_availability_sender);

    let window = ApplicationWindow::builder()
        .application(application)
        .title("Mullvad-GTK")
        .default_width(320)
        .default_height(568)
        .resizable(false)
        .child(&stack)
        .build();
    window.set_titlebar(Some(&header_bar));
    gtk::glib::spawn_future_local({
        let tray_available = Rc::clone(&tray_available);
        let window = window.clone();
        async move {
            while let Ok(available) = tray_availability_receiver.recv().await {
                tray_available.set(available);
                if !available && !window.is_visible() {
                    window.present();
                }
            }
        }
    });
    window.connect_close_request({
        let window = window.clone();
        let tray_available = Rc::clone(&tray_available);
        move |_| {
            if tray_available.get() {
                window.hide();
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        }
    });

    let (tunnel_sender, tunnel_receiver) = async_channel::unbounded::<TunnelStatus>();
    let (settings_sender, settings_receiver) =
        async_channel::unbounded::<Result<AppSettings, String>>();
    let (account_sender, account_receiver) =
        async_channel::unbounded::<Result<AccountStatus, String>>();
    let (relay_sender, relay_receiver) =
        async_channel::unbounded::<Result<Vec<RelayLocation>, String>>();
    let (split_sender, split_receiver) = async_channel::unbounded::<Result<Vec<i32>, String>>();
    let (advanced_sender, advanced_receiver) =
        async_channel::unbounded::<Result<AdvancedSettings, String>>();
    let (voucher_sender, voucher_receiver) = async_channel::unbounded::<Result<String, String>>();
    let (devices_sender, devices_receiver) =
        async_channel::unbounded::<Result<Vec<DeviceSummary>, String>>();
    let displayed_status = Rc::new(RefCell::new(TunnelStatus::Disconnecting));
    let relay_locations = Rc::new(RefCell::new(Vec::<RelayLocation>::new()));
    let active_account_number = Rc::new(RefCell::new(None::<String>));

    action_button.connect_clicked({
        let controller = Arc::clone(&controller);
        let displayed_status = Rc::clone(&displayed_status);
        let runtime = Arc::clone(&runtime);
        let sender = tunnel_sender.clone();
        move |button| {
            button.set_sensitive(false);
            let command = if displayed_status.borrow().wants_disconnect() {
                DaemonCommand::Disconnect
            } else {
                DaemonCommand::Connect
            };
            spawn_tunnel_command(&runtime, &controller, &sender, command);
        }
    });
    reconnect_button.connect_clicked({
        let controller = Arc::clone(&controller);
        let runtime = Arc::clone(&runtime);
        let sender = tunnel_sender.clone();
        move |button| {
            button.set_sensitive(false);
            spawn_tunnel_command(&runtime, &controller, &sender, DaemonCommand::Reconnect);
        }
    });

    home_button.connect_clicked({
        let stack = stack.clone();
        move |_| stack.set_visible_child_name("main")
    });
    location_button.connect_clicked({
        let stack = stack.clone();
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = relay_sender.clone();
        move |_| {
            stack.set_visible_child_name("locations");
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            runtime.spawn(async move {
                let _ = sender.send(daemon.relay_locations().await).await;
            });
        }
    });
    settings_button.connect_clicked({
        let stack = stack.clone();
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = settings_sender.clone();
        move |_| {
            stack.set_visible_child_name("settings");
            load_settings(&runtime, &daemon, &sender);
        }
    });
    account_button.connect_clicked({
        let stack = stack.clone();
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = account_sender.clone();
        move |_| {
            stack.set_visible_child_name("account");
            load_account(&runtime, &daemon, &sender);
        }
    });
    split_tunnel_button.connect_clicked({
        let stack = stack.clone();
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = split_sender.clone();
        move |_| {
            stack.set_visible_child_name("split-tunnel");
            load_split_tunnel(&runtime, &daemon, &sender);
        }
    });
    advanced_settings_button.connect_clicked({
        let stack = stack.clone();
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = advanced_sender.clone();
        move |_| {
            stack.set_visible_child_name("advanced");
            load_advanced_settings(&runtime, &daemon, &sender);
        }
    });
    support_button.connect_clicked({
        let stack = stack.clone();
        move |_| stack.set_visible_child_name("support")
    });
    report_issue_button.connect_clicked({
        let error = support_error.clone();
        move |_| {
            open_uri(
                "https://github.com/Greenstorm5417/Mullvad-GTK/issues/new/choose",
                &error,
            );
        }
    });
    open_issues_button.connect_clicked({
        let error = support_error.clone();
        move |_| {
            open_uri(
                "https://github.com/Greenstorm5417/Mullvad-GTK/issues",
                &error,
            );
        }
    });

    for (switch, setting) in [
        (&allow_lan, BooleanSetting::AllowLan),
        (&auto_connect, BooleanSetting::AutoConnect),
        (&enable_ipv6, BooleanSetting::EnableIpv6),
        (&lockdown_mode, BooleanSetting::LockdownMode),
        (&show_beta, BooleanSetting::ShowBetaReleases),
    ] {
        wire_setting_switch(
            switch,
            setting,
            Arc::clone(&runtime),
            Arc::clone(&daemon),
            settings_sender.clone(),
        );
    }
    autostart.connect_state_set({
        let error_label = settings_error.clone();
        move |_switch, enabled| {
            match desktop::set_autostart(enabled) {
                Ok(()) => error_label.set_label(""),
                Err(error) => error_label.set_label(&error),
            }
            gtk::glib::Propagation::Proceed
        }
    });
    enable_notifications.connect_state_set({
        let error_label = settings_error.clone();
        move |_switch, enabled| {
            match desktop::set_notifications_enabled(enabled) {
                Ok(()) => error_label.set_label(""),
                Err(error) => error_label.set_label(&error),
            }
            gtk::glib::Propagation::Proceed
        }
    });
    monochromatic_icon.connect_state_set({
        let error_label = settings_error.clone();
        let tray_status_sender = tray_status_sender.clone();
        move |_switch, enabled| {
            match desktop::set_monochromatic_enabled(enabled) {
                Ok(()) => {
                    error_label.set_label("");
                    let _ = tray_status_sender.try_send(TrayUpdate::Monochromatic(enabled));
                }
                Err(error) => error_label.set_label(&error),
            }
            gtk::glib::Propagation::Proceed
        }
    });
    for (switch, setting) in [
        (&block_ads, AdvancedToggle::Dns(DnsBlocker::Ads)),
        (&block_trackers, AdvancedToggle::Dns(DnsBlocker::Trackers)),
        (&block_malware, AdvancedToggle::Dns(DnsBlocker::Malware)),
        (&block_adult, AdvancedToggle::Dns(DnsBlocker::AdultContent)),
        (&block_gambling, AdvancedToggle::Dns(DnsBlocker::Gambling)),
        (&block_social, AdvancedToggle::Dns(DnsBlocker::SocialMedia)),
        (&quantum_resistant, AdvancedToggle::QuantumResistance),
        (&daita, AdvancedToggle::Daita),
    ] {
        wire_advanced_switch(
            switch,
            setting,
            Arc::clone(&runtime),
            Arc::clone(&daemon),
            advanced_sender.clone(),
        );
    }
    apply_advanced.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = advanced_sender.clone();
        let multihop = multihop.clone();
        let obfuscation = obfuscation.clone();
        let mtu = mtu.clone();
        move |button| {
            button.set_sensitive(false);
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            let multihop = multihop_mode(multihop.selected());
            let obfuscation = obfuscation_mode(obfuscation.selected());
            let mtu_value = u32::try_from(mtu.value_as_int())
                .ok()
                .filter(|value| *value > 0);
            runtime.spawn(async move {
                let result = async {
                    daemon.set_mtu(mtu_value).await?;
                    daemon.set_multihop(multihop).await?;
                    daemon.set_obfuscation(obfuscation).await?;
                    daemon.advanced_settings().await
                }
                .await;
                let _ = sender.send(result).await;
            });
        }
    });

    login_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = account_sender.clone();
        let entry = account_entry.clone();
        move |_| {
            let account_number = entry.text().replace(' ', "");
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            runtime.spawn(async move {
                let result = match daemon.login(account_number).await {
                    Ok(()) => daemon.account_status().await,
                    Err(error) => Err(error),
                };
                let _ = sender.send(result).await;
            });
        }
    });
    create_account_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = account_sender.clone();
        move |_| {
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            runtime.spawn(async move {
                let result = match daemon.create_account().await {
                    Ok(_) => daemon.account_status().await,
                    Err(error) => Err(error),
                };
                let _ = sender.send(result).await;
            });
        }
    });
    logout_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = account_sender.clone();
        move |_| {
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            runtime.spawn(async move {
                let result = match daemon.logout().await {
                    Ok(()) => daemon.account_status().await,
                    Err(error) => Err(error),
                };
                let _ = sender.send(result).await;
            });
        }
    });
    redeem_voucher_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = voucher_sender.clone();
        let entry = voucher_entry.clone();
        move |button| {
            button.set_sensitive(false);
            let voucher = entry.text().trim().to_owned();
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            runtime.spawn(async move {
                let _ = sender.send(daemon.submit_voucher(voucher).await).await;
            });
        }
    });
    manage_devices_button.connect_clicked({
        let stack = stack.clone();
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = devices_sender.clone();
        let account_number = Rc::clone(&active_account_number);
        move |_| {
            let Some(account_number) = account_number.borrow().clone() else {
                return;
            };
            stack.set_visible_child_name("devices");
            load_devices(&runtime, &daemon, &sender, account_number);
        }
    });
    refresh_devices_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = devices_sender.clone();
        let account_number = Rc::clone(&active_account_number);
        move |_| {
            if let Some(account_number) = account_number.borrow().clone() {
                load_devices(&runtime, &daemon, &sender, account_number);
            }
        }
    });
    remove_device_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = devices_sender.clone();
        let account_number = Rc::clone(&active_account_number);
        let device_id_entry = device_id_entry.clone();
        move |button| {
            let Some(account_number) = account_number.borrow().clone() else {
                return;
            };
            button.set_sensitive(false);
            let device_id = device_id_entry.text().trim().to_owned();
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            runtime.spawn(async move {
                let result = match daemon
                    .remove_device(account_number.clone(), device_id)
                    .await
                {
                    Ok(()) => daemon.devices(account_number).await,
                    Err(error) => Err(error),
                };
                let _ = sender.send(result).await;
            });
        }
    });

    apply_relay_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let relay_locations = Rc::clone(&relay_locations);
        let dropdown = relay_dropdown.clone();
        let error = relay_error.clone();
        let stack = stack.clone();
        move |_| {
            let Some(relay) = relay_locations
                .borrow()
                .get(dropdown.selected() as usize)
                .cloned()
            else {
                error.set_label("Select a relay first");
                return;
            };
            let daemon = Arc::clone(&daemon);
            let error = error.clone();
            let stack = stack.clone();
            let (sender, receiver) = async_channel::bounded(1);
            runtime.spawn(async move {
                let _ = sender.send(daemon.select_relay(relay).await).await;
            });
            gtk::glib::spawn_future_local(async move {
                if let Ok(result) = receiver.recv().await {
                    match result {
                        Ok(()) => stack.set_visible_child_name("main"),
                        Err(message) => error.set_label(&message),
                    }
                }
            });
        }
    });

    wire_split_process_button(
        &add_process_button,
        &process_id,
        true,
        Arc::clone(&runtime),
        Arc::clone(&daemon),
        split_sender.clone(),
    );
    wire_split_process_button(
        &remove_process_button,
        &process_id,
        false,
        Arc::clone(&runtime),
        Arc::clone(&daemon),
        split_sender.clone(),
    );
    clear_processes_button.connect_clicked({
        let runtime = Arc::clone(&runtime);
        let daemon = Arc::clone(&daemon);
        let sender = split_sender.clone();
        move |_| {
            let daemon = Arc::clone(&daemon);
            let sender = sender.clone();
            runtime.spawn(async move {
                let result = match daemon.clear_split_tunnel_processes().await {
                    Ok(()) => daemon
                        .split_tunnel_state()
                        .await
                        .map(|state| state.process_ids),
                    Err(error) => Err(error),
                };
                let _ = sender.send(result).await;
            });
        }
    });

    gtk::glib::spawn_future_local({
        let displayed_status = Rc::clone(&displayed_status);
        let previous_status = Rc::new(RefCell::new(None::<TunnelStatus>));
        let window = window.clone();
        let runtime = Arc::clone(&runtime);
        async move {
            while let Ok(status) = tunnel_receiver.recv().await {
                render_status(
                    &status,
                    &status_label,
                    &detail_label,
                    &spinner,
                    &action_button,
                    &reconnect_button,
                );
                map_view.set_status(&status);
                let _ = tray_status_sender.try_send(TrayUpdate::Status(status.clone()));
                if previous_status
                    .borrow()
                    .as_ref()
                    .is_some_and(|previous| previous != &status)
                    && !window.is_active()
                    && enable_notifications.is_active()
                {
                    let notification_status = status.clone();
                    runtime.spawn_blocking(move || {
                        desktop::notify_tunnel_status(&notification_status);
                    });
                }
                previous_status.replace(Some(status.clone()));
                displayed_status.replace(status);
            }
        }
    });
    gtk::glib::spawn_future_local(async move {
        while let Ok(result) = settings_receiver.recv().await {
            match result {
                Ok(settings) => {
                    settings_error.set_label("");
                    apply_settings(
                        &settings,
                        &allow_lan,
                        &auto_connect,
                        &enable_ipv6,
                        &lockdown_mode,
                        &show_beta,
                    );
                }
                Err(error) => settings_error.set_label(&error),
            }
        }
    });

    gtk::glib::spawn_future_local({
        let application = application.clone();
        let window = window.clone();
        let stack = stack.clone();
        let runtime = Arc::clone(&runtime);
        let controller = Arc::clone(&controller);
        let tunnel_sender = tunnel_sender.clone();
        async move {
            while let Ok(command) = desktop_command_receiver.recv().await {
                match command {
                    DesktopCommand::Show => {
                        stack.set_visible_child_name("main");
                        window.present();
                    }
                    DesktopCommand::Connect => spawn_tunnel_command(
                        &runtime,
                        &controller,
                        &tunnel_sender,
                        DaemonCommand::Connect,
                    ),
                    DesktopCommand::Reconnect => spawn_tunnel_command(
                        &runtime,
                        &controller,
                        &tunnel_sender,
                        DaemonCommand::Reconnect,
                    ),
                    DesktopCommand::Disconnect => spawn_tunnel_command(
                        &runtime,
                        &controller,
                        &tunnel_sender,
                        DaemonCommand::Disconnect,
                    ),
                    DesktopCommand::DisconnectAndQuit => {
                        let (done_sender, done_receiver) = async_channel::bounded(1);
                        let controller = Arc::clone(&controller);
                        runtime.spawn(async move {
                            let _ = controller.execute(DaemonCommand::Disconnect).await;
                            let _ = done_sender.send(()).await;
                        });
                        let _ = done_receiver.recv().await;
                        application.quit();
                    }
                }
            }
        }
    });
    let account_redeem_voucher_button = redeem_voucher_button.clone();
    gtk::glib::spawn_future_local(async move {
        let widgets = AccountWidgets {
            state: account_state,
            detail: account_detail,
            entry: account_entry,
            login: login_button,
            create: create_account_button,
            logout: logout_button,
            voucher_entry: voucher_entry.clone(),
            redeem_voucher: account_redeem_voucher_button,
            manage_devices: manage_devices_button.clone(),
            error: account_error,
            account_number: Rc::clone(&active_account_number),
        };
        while let Ok(result) = account_receiver.recv().await {
            render_account(result, &widgets);
        }
    });
    gtk::glib::spawn_future_local({
        let relay_locations = Rc::clone(&relay_locations);
        async move {
            while let Ok(result) = relay_receiver.recv().await {
                match result {
                    Ok(locations) => {
                        relay_error.set_label("");
                        let labels: Vec<&str> = locations
                            .iter()
                            .map(|location| location.label.as_str())
                            .collect();
                        relay_model.splice(0, relay_model.n_items(), &labels);
                        relay_locations.replace(locations);
                    }
                    Err(error) => relay_error.set_label(&error),
                }
            }
        }
    });
    gtk::glib::spawn_future_local(async move {
        while let Ok(result) = split_receiver.recv().await {
            match result {
                Ok(process_ids) => {
                    split_error.set_label("");
                    excluded_processes.set_label(&format_process_ids(&process_ids));
                }
                Err(error) => split_error.set_label(&error),
            }
        }
    });
    gtk::glib::spawn_future_local(async move {
        while let Ok(result) = advanced_receiver.recv().await {
            apply_advanced.set_sensitive(true);
            match result {
                Ok(settings) => {
                    advanced_error.set_label("");
                    for (switch, enabled) in [
                        (&block_ads, settings.block_ads),
                        (&block_trackers, settings.block_trackers),
                        (&block_malware, settings.block_malware),
                        (&block_adult, settings.block_adult_content),
                        (&block_gambling, settings.block_gambling),
                        (&block_social, settings.block_social_media),
                        (&quantum_resistant, settings.quantum_resistant),
                        (&daita, settings.daita),
                    ] {
                        switch.set_active(enabled);
                        switch.set_sensitive(true);
                    }
                    multihop.set_selected(match settings.multihop {
                        MultihopMode::Auto => 0,
                        MultihopMode::Always => 1,
                        MultihopMode::Never => 2,
                    });
                    obfuscation.set_selected(match settings.obfuscation {
                        ObfuscationMode::Auto => 0,
                        ObfuscationMode::Off => 1,
                        ObfuscationMode::WireguardPort => 2,
                        ObfuscationMode::UdpOverTcp => 3,
                        ObfuscationMode::Shadowsocks => 4,
                        ObfuscationMode::Quic => 5,
                        ObfuscationMode::Lwo => 6,
                    });
                    mtu.set_value(f64::from(settings.mtu.unwrap_or(0)));
                }
                Err(error) => advanced_error.set_label(&error),
            }
        }
    });
    gtk::glib::spawn_future_local(async move {
        while let Ok(result) = voucher_receiver.recv().await {
            redeem_voucher_button.set_sensitive(true);
            match result {
                Ok(message) => voucher_status.set_label(&message),
                Err(error) => voucher_status.set_label(&error),
            }
        }
    });
    gtk::glib::spawn_future_local(async move {
        while let Ok(result) = devices_receiver.recv().await {
            remove_device_button.set_sensitive(true);
            match result {
                Ok(devices) => {
                    devices_error.set_label("");
                    devices_label.set_label(&format_devices(&devices));
                }
                Err(error) => devices_error.set_label(&error),
            }
        }
    });

    let event_daemon = daemon.as_ref().clone();
    runtime.spawn(async move {
        loop {
            let status = controller.execute(DaemonCommand::Refresh).await;
            if tunnel_sender.send(status).await.is_err() {
                break;
            }
            if let Err(error) = event_daemon
                .listen_tunnel_statuses(tunnel_sender.clone())
                .await
                && tunnel_sender
                    .send(TunnelStatus::Unavailable(error))
                    .await
                    .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    if !start_in_background {
        window.present();
    }
}

fn spawn_tunnel_command(
    runtime: &tokio::runtime::Runtime,
    controller: &Arc<Controller<MullvadDaemon>>,
    sender: &async_channel::Sender<TunnelStatus>,
    command: DaemonCommand,
) {
    let controller = Arc::clone(controller);
    let sender = sender.clone();
    runtime.spawn(async move {
        let _ = sender.send(controller.execute(command).await).await;
    });
}

fn load_settings(
    runtime: &tokio::runtime::Runtime,
    daemon: &Arc<MullvadDaemon>,
    sender: &async_channel::Sender<Result<AppSettings, String>>,
) {
    let daemon = Arc::clone(daemon);
    let sender = sender.clone();
    runtime.spawn(async move {
        let _ = sender.send(daemon.settings().await).await;
    });
}

fn load_account(
    runtime: &tokio::runtime::Runtime,
    daemon: &Arc<MullvadDaemon>,
    sender: &async_channel::Sender<Result<AccountStatus, String>>,
) {
    let daemon = Arc::clone(daemon);
    let sender = sender.clone();
    runtime.spawn(async move {
        let _ = sender.send(daemon.account_status().await).await;
    });
}

fn load_split_tunnel(
    runtime: &tokio::runtime::Runtime,
    daemon: &Arc<MullvadDaemon>,
    sender: &async_channel::Sender<Result<Vec<i32>, String>>,
) {
    let daemon = Arc::clone(daemon);
    let sender = sender.clone();
    runtime.spawn(async move {
        let result = daemon
            .split_tunnel_state()
            .await
            .map(|state| state.process_ids);
        let _ = sender.send(result).await;
    });
}

fn load_advanced_settings(
    runtime: &tokio::runtime::Runtime,
    daemon: &Arc<MullvadDaemon>,
    sender: &async_channel::Sender<Result<AdvancedSettings, String>>,
) {
    let daemon = Arc::clone(daemon);
    let sender = sender.clone();
    runtime.spawn(async move {
        let _ = sender.send(daemon.advanced_settings().await).await;
    });
}

fn load_devices(
    runtime: &tokio::runtime::Runtime,
    daemon: &Arc<MullvadDaemon>,
    sender: &async_channel::Sender<Result<Vec<DeviceSummary>, String>>,
    account_number: String,
) {
    let daemon = Arc::clone(daemon);
    let sender = sender.clone();
    runtime.spawn(async move {
        let _ = sender.send(daemon.devices(account_number).await).await;
    });
}

fn wire_setting_switch(
    switch: &gtk::Switch,
    setting: BooleanSetting,
    runtime: Arc<tokio::runtime::Runtime>,
    daemon: Arc<MullvadDaemon>,
    sender: async_channel::Sender<Result<AppSettings, String>>,
) {
    switch.connect_state_set(move |switch, enabled| {
        switch.set_sensitive(false);
        let daemon = Arc::clone(&daemon);
        let sender = sender.clone();
        runtime.spawn(async move {
            let result = match daemon.set_boolean_setting(setting, enabled).await {
                Ok(()) => daemon.settings().await,
                Err(error) => Err(error),
            };
            let _ = sender.send(result).await;
        });
        gtk::glib::Propagation::Proceed
    });
}

fn wire_advanced_switch(
    switch: &gtk::Switch,
    setting: AdvancedToggle,
    runtime: Arc<tokio::runtime::Runtime>,
    daemon: Arc<MullvadDaemon>,
    sender: async_channel::Sender<Result<AdvancedSettings, String>>,
) {
    switch.connect_state_set(move |switch, enabled| {
        switch.set_sensitive(false);
        let daemon = Arc::clone(&daemon);
        let sender = sender.clone();
        runtime.spawn(async move {
            let mutation = match setting {
                AdvancedToggle::Dns(blocker) => daemon.set_dns_blocker(blocker, enabled).await,
                AdvancedToggle::QuantumResistance => daemon.set_quantum_resistant(enabled).await,
                AdvancedToggle::Daita => daemon.set_daita(enabled).await,
            };
            let result = match mutation {
                Ok(()) => daemon.advanced_settings().await,
                Err(error) => Err(error),
            };
            let _ = sender.send(result).await;
        });
        gtk::glib::Propagation::Proceed
    });
}

fn wire_split_process_button(
    button: &gtk::Button,
    process_id: &gtk::SpinButton,
    exclude: bool,
    runtime: Arc<tokio::runtime::Runtime>,
    daemon: Arc<MullvadDaemon>,
    sender: async_channel::Sender<Result<Vec<i32>, String>>,
) {
    let process_id = process_id.clone();
    button.connect_clicked(move |_| {
        let process_id = process_id.value_as_int();
        let daemon = Arc::clone(&daemon);
        let sender = sender.clone();
        runtime.spawn(async move {
            let mutation = if exclude {
                daemon.add_split_tunnel_process(process_id).await
            } else {
                daemon.remove_split_tunnel_process(process_id).await
            };
            let result = match mutation {
                Ok(()) => daemon
                    .split_tunnel_state()
                    .await
                    .map(|state| state.process_ids),
                Err(error) => Err(error),
            };
            let _ = sender.send(result).await;
        });
    });
}

fn render_status(
    status: &TunnelStatus,
    status_label: &gtk::Label,
    detail_label: &gtk::Label,
    spinner: &gtk::Spinner,
    action_button: &gtk::Button,
    reconnect_button: &gtk::Button,
) {
    status_label.set_label(status.headline());
    detail_label.set_label(&status.detail());
    action_button.set_label(status.action_label());
    action_button.set_sensitive(!matches!(
        status,
        TunnelStatus::Disconnecting | TunnelStatus::Unavailable(_)
    ));
    spinner.set_spinning(status.is_busy());
    spinner.set_visible(status.is_busy());
    reconnect_button.set_visible(matches!(status, TunnelStatus::Connected { .. }));
    reconnect_button.set_sensitive(matches!(status, TunnelStatus::Connected { .. }));

    for class in STATUS_CLASSES {
        status_label.remove_css_class(class);
    }
    status_label.add_css_class(status.style_class());
    if status.wants_disconnect() {
        action_button.add_css_class("destructive-action");
        action_button.remove_css_class("suggested-action");
    } else {
        action_button.remove_css_class("destructive-action");
        action_button.add_css_class("suggested-action");
    }
}

fn render_account(result: Result<AccountStatus, String>, widgets: &AccountWidgets) {
    widgets.error.set_label("");
    match result {
        Ok(AccountStatus::LoggedIn {
            account_number,
            device_name,
            expiry,
        }) => {
            widgets.account_number.replace(Some(account_number.clone()));
            widgets.state.set_label("Logged in");
            let expiry = expiry.unwrap_or_else(|| "Paid-until date unavailable".to_owned());
            widgets.detail.set_label(&format!(
                "Device: {device_name}\nAccount: {account_number}\n{expiry}"
            ));
            widgets.entry.set_visible(false);
            widgets.login.set_visible(false);
            widgets.create.set_visible(false);
            widgets.logout.set_visible(true);
            widgets.voucher_entry.set_visible(true);
            widgets.redeem_voucher.set_visible(true);
            widgets.manage_devices.set_visible(true);
        }
        Ok(AccountStatus::LoggedOut) => {
            widgets.account_number.replace(None);
            widgets.state.set_label("Log in to Mullvad VPN");
            widgets.detail.set_label("");
            widgets.entry.set_visible(true);
            widgets.login.set_visible(true);
            widgets.create.set_visible(true);
            widgets.logout.set_visible(false);
            widgets.voucher_entry.set_visible(false);
            widgets.redeem_voucher.set_visible(false);
            widgets.manage_devices.set_visible(false);
        }
        Ok(AccountStatus::Revoked) => {
            widgets.account_number.replace(None);
            widgets.state.set_label("This device has been revoked");
            widgets
                .detail
                .set_label("Log in again to register this device.");
            widgets.entry.set_visible(true);
            widgets.login.set_visible(true);
            widgets.create.set_visible(false);
            widgets.logout.set_visible(false);
            widgets.voucher_entry.set_visible(false);
            widgets.redeem_voucher.set_visible(false);
            widgets.manage_devices.set_visible(false);
        }
        Err(message) => widgets.error.set_label(&message),
    }
}

fn apply_settings(
    settings: &AppSettings,
    allow_lan: &gtk::Switch,
    auto_connect: &gtk::Switch,
    enable_ipv6: &gtk::Switch,
    lockdown_mode: &gtk::Switch,
    show_beta: &gtk::Switch,
) {
    for (switch, enabled) in [
        (allow_lan, settings.allow_lan),
        (auto_connect, settings.auto_connect),
        (enable_ipv6, settings.enable_ipv6),
        (lockdown_mode, settings.lockdown_mode),
        (show_beta, settings.show_beta_releases),
    ] {
        switch.set_active(enabled);
        switch.set_sensitive(true);
    }
}

fn format_process_ids(process_ids: &[i32]) -> String {
    if process_ids.is_empty() {
        "No excluded processes".to_owned()
    } else {
        format!(
            "Excluded process IDs: {}",
            process_ids
                .iter()
                .map(i32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn format_devices(devices: &[DeviceSummary]) -> String {
    if devices.is_empty() {
        "No devices found".to_owned()
    } else {
        devices
            .iter()
            .map(|device| format!("{}\n{}", device.name, device.id))
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

fn multihop_mode(selected: u32) -> MultihopMode {
    match selected {
        1 => MultihopMode::Always,
        2 => MultihopMode::Never,
        _ => MultihopMode::Auto,
    }
}

fn obfuscation_mode(selected: u32) -> ObfuscationMode {
    match selected {
        1 => ObfuscationMode::Off,
        2 => ObfuscationMode::WireguardPort,
        3 => ObfuscationMode::UdpOverTcp,
        4 => ObfuscationMode::Shadowsocks,
        5 => ObfuscationMode::Quic,
        6 => ObfuscationMode::Lwo,
        _ => ObfuscationMode::Auto,
    }
}

fn switch_row(title: &str) -> (gtk::ListBoxRow, gtk::Switch) {
    let label = gtk::Label::builder()
        .label(title)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    let switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.set_margin_top(10);
    content.set_margin_bottom(10);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&label);
    content.append(&switch);
    let row = gtk::ListBoxRow::builder().child(&content).build();
    (row, switch)
}

fn page_content() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .halign(gtk::Align::Fill)
        .valign(gtk::Align::Start)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build()
}

fn scroll_page(content: &gtk::Box) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .child(content)
        .build()
}

fn page_title(title: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(title)
        .css_classes(["page-title"])
        .halign(gtk::Align::Start)
        .build()
}

fn field_label(title: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(title)
        .css_classes(["field-label"])
        .halign(gtk::Align::Start)
        .build()
}

fn error_label() -> gtk::Label {
    gtk::Label::builder()
        .css_classes(["error-message"])
        .wrap(true)
        .build()
}

fn open_uri(uri: &str, error: &gtk::Label) {
    match gtk::gio::AppInfo::launch_default_for_uri(uri, None::<&gtk::gio::AppLaunchContext>) {
        Ok(()) => error.set_label(""),
        Err(reason) => error.set_label(&format!("Could not open the link: {reason}")),
    }
}

fn icon_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon_name)
        .tooltip_text(tooltip)
        .build()
}

fn embedded_svg(bytes: &[u8], width: i32, height: i32) -> gtk::Picture {
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader
        .write(bytes)
        .expect("embedded Mullvad SVG should decode");
    loader
        .close()
        .expect("embedded Mullvad SVG should finish decoding");
    let texture = gtk::gdk::Texture::for_pixbuf(
        &loader
            .pixbuf()
            .expect("embedded Mullvad SVG should contain a pixbuf"),
    );
    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_size_request(width, height);
    picture.set_can_shrink(true);
    picture
}

fn install_styles() {
    let provider = gtk::CssProvider::new();
    provider.load_from_data(include_str!("style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("GTK display should be available"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
