# Linux desktop compatibility

Mullvad-Gui-Slint relies on freedesktop and Slint interfaces rather than a specific
desktop shell.

- The application window works on X11 and Wayland compositors supported by Slint's
  winit backend. The desktop entry uses the executable WM class so launchers can
  associate the window on traditional, tiling, and Wayland compositors.
- Web links use the system's default URI handler.
- The tray uses the StatusNotifierItem protocol. KDE Plasma and compatible hosts
  support it directly; GNOME typically requires an AppIndicator extension.
- Close-to-tray is enabled only after a StatusNotifier host announces itself and
  accepts the indicator. Merely registering on D-Bus is not treated as tray
  availability. Without a host, closing the window exits normally.
- Notifications use the standard desktop notification D-Bus interface. Tunnel
  operation and in-app status remain available if no notification service runs.
- Autostart uses the XDG autostart specification and records the installed
  executable or current AppImage path. Reserved desktop-entry command characters
  are escaped, so paths containing spaces, quotes, or shell metacharacters work.

No application can force a shell to expose a tray or notification service. The
fallback behavior ensures those optional facilities never make the VPN controls
unreachable.
