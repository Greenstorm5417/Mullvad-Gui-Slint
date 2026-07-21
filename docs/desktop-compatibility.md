# Linux desktop compatibility

Mullvad-GTK relies on freedesktop and GTK interfaces rather than a specific
desktop shell.

- The application window works on X11 and Wayland compositors supported by GTK4.
- Web links use GIO's default URI handler and can be routed through a desktop portal.
- The tray uses the StatusNotifierItem protocol. KDE Plasma and compatible hosts
  support it directly; GNOME typically requires an AppIndicator extension.
- Close-to-tray is enabled only after a tray host accepts the indicator. Without
  one, closing the window exits the application normally.
- Notifications use the standard desktop notification D-Bus interface. Tunnel
  operation and in-app status remain available if no notification service runs.
- Autostart uses the XDG autostart specification and records the installed
  executable or current AppImage path.

No application can force a shell to expose a tray or notification service. The
fallback behavior ensures those optional facilities never make the VPN controls
unreachable.
