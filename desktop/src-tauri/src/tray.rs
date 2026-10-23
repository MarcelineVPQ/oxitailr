//! The system tray: an icon with a small menu, and the window behaviour that
//! goes with it (close to tray, start minimized, launch at login).
//!
//! Two settings live in `desktop-settings.json` in the oxitailr config
//! directory. Launch at login is not stored there: the autostart plugin reads
//! the operating system's own registration, so it is the one source of truth.
//!
//! Every show or hide of the main window emits `window-visible` with a bool,
//! so the page can stop drawing while nobody can see it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, Window, WindowEvent, Wry};
use tauri_plugin_autostart::{AutoLaunchManager, ManagerExt};

/// The window the tray shows and hides.
const MAIN: &str = "main";
/// Event sent to the page whenever the main window is shown or hidden.
const VISIBLE_EVENT: &str = "window-visible";
/// Command-line flag the autostart entry passes: start in the tray.
const HIDDEN_FLAG: &str = "--hidden";
const SETTINGS_FILE: &str = "desktop-settings.json";

const ID_TOGGLE: &str = "tray-toggle";
const ID_STATUS: &str = "tray-status";
const ID_CLOSE_TO_TRAY: &str = "tray-close-to-tray";
const ID_START_MINIMIZED: &str = "tray-start-minimized";
const ID_LAUNCH_AT_LOGIN: &str = "tray-launch-at-login";
const ID_NOTIFY: &str = "tray-notify";
const ID_QUIT: &str = "tray-quit";

/// Desktop-only settings the tray menu edits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopSettings {
    /// Closing the window hides it to the tray instead of quitting.
    pub close_to_tray: bool,
    /// The window starts hidden in the tray.
    pub start_minimized: bool,
    /// Alerts raise a desktop notification while the window isn't focused.
    pub notify_alerts: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            close_to_tray: true,
            start_minimized: false,
            notify_alerts: true,
        }
    }
}

/// Handles kept for the life of the app.
struct TrayState {
    /// Keeps the tray icon alive; dropping it removes the icon. None when the
    /// desktop has no tray host (stock GNOME, a bare window manager): the app
    /// runs without one, and nothing hides the window where it can't be got back.
    tray: Option<TrayIcon>,
    toggle: MenuItem<Wry>,
    status: MenuItem<Wry>,
    close_to_tray: CheckMenuItem<Wry>,
    start_minimized: CheckMenuItem<Wry>,
    launch_at_login: CheckMenuItem<Wry>,
    notify: CheckMenuItem<Wry>,
    settings: Mutex<DesktopSettings>,
    settings_path: PathBuf,
}

impl TrayState {
    fn settings(&self) -> DesktopSettings {
        self.settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn update(&self, change: impl FnOnce(&mut DesktopSettings)) {
        let mut guard = self.settings.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut guard);
        if let Err(e) = save_settings_to(&self.settings_path, &guard) {
            tracing::warn!("could not save {}: {e}", self.settings_path.display());
        }
    }
}

fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("oxitailr")
        .join(SETTINGS_FILE)
}

/// Read the settings at `path`. A missing file gives the defaults. A file
/// that cannot be read as settings is moved aside to `<name>.corrupt` (so the
/// next save does not destroy it) and the defaults are used.
fn load_settings_from(path: &Path) -> DesktopSettings {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DesktopSettings::default(),
        Err(e) => {
            tracing::warn!("could not read {}: {e}; using defaults", path.display());
            return DesktopSettings::default();
        }
    };
    match serde_json::from_str(&text) {
        Ok(settings) => settings,
        Err(e) => {
            let aside = corrupt_path(path);
            tracing::warn!(
                "{} is not valid settings ({e}); moved to {} and using defaults",
                path.display(),
                aside.display()
            );
            if let Err(e) = std::fs::rename(path, &aside) {
                tracing::warn!("could not move {} aside: {e}", path.display());
            }
            DesktopSettings::default()
        }
    }
}

fn corrupt_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".corrupt");
    PathBuf::from(name)
}

/// Write the settings to `path` atomically: a temp file beside it, then a
/// rename over the old one, so a crash mid-write never leaves half a file.
fn save_settings_to(path: &Path, settings: &DesktopSettings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn toggle_label(visible: bool) -> &'static str {
    if visible {
        "Hide Oxitailr"
    } else {
        "Show Oxitailr"
    }
}

fn autolaunch(app: &AppHandle) -> Option<tauri::State<'_, AutoLaunchManager>> {
    // `autolaunch()` panics when the plugin is not registered; check first.
    app.try_state::<AutoLaunchManager>()?;
    Some(app.autolaunch())
}

/// Build the tray and its menu, load the settings, and show the main window
/// unless this launch should start hidden (`--hidden` or "Start minimized").
pub fn setup(app: &App) -> tauri::Result<()> {
    let handle = app.handle();
    let path = settings_path();
    let settings = load_settings_from(&path);
    let start_hidden = settings.start_minimized || std::env::args().any(|a| a == HIDDEN_FLAG);

    let login_enabled = match autolaunch(handle).map(|m| m.is_enabled()) {
        Some(Ok(on)) => on,
        Some(Err(e)) => {
            tracing::warn!("could not read launch-at-login state: {e}");
            false
        }
        None => false,
    };

    let toggle = MenuItem::with_id(
        app,
        ID_TOGGLE,
        toggle_label(!start_hidden),
        true,
        None::<&str>,
    )?;
    let status = MenuItem::with_id(app, ID_STATUS, "Idle", false, None::<&str>)?;
    let close_to_tray = CheckMenuItem::with_id(
        app,
        ID_CLOSE_TO_TRAY,
        "Close to tray",
        true,
        settings.close_to_tray,
        None::<&str>,
    )?;
    let start_minimized = CheckMenuItem::with_id(
        app,
        ID_START_MINIMIZED,
        "Start minimized",
        true,
        settings.start_minimized,
        None::<&str>,
    )?;
    let launch_at_login = CheckMenuItem::with_id(
        app,
        ID_LAUNCH_AT_LOGIN,
        "Launch at login",
        true,
        login_enabled,
        None::<&str>,
    )?;
    let notify = CheckMenuItem::with_id(
        app,
        ID_NOTIFY,
        "Notify me of alerts",
        true,
        settings.notify_alerts,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &status,
            &PredefinedMenuItem::separator(app)?,
            &close_to_tray,
            &start_minimized,
            &launch_at_login,
            &notify,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("Oxitailr")
        .menu(&menu)
        // Left click toggles the window; the menu is on right click.
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(on_tray_event);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = match builder.build(app) {
        Ok(tray) => Some(tray),
        Err(e) => {
            tracing::warn!("no system tray ({e}); running without one");
            None
        }
    };
    let has_tray = tray.is_some();

    app.manage(TrayState {
        tray,
        toggle,
        status,
        close_to_tray,
        start_minimized,
        launch_at_login,
        notify,
        settings: Mutex::new(settings),
        settings_path: path,
    });

    // Without a tray a hidden window couldn't be brought back: always shown.
    if start_hidden && has_tray {
        hide_main(handle);
    } else {
        show_main(handle);
    }
    Ok(())
}

/// Closing the main window hides it to the tray when "Close to tray" is on;
/// otherwise the close goes ahead as normal.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    if window.label() != MAIN {
        return;
    }
    if let WindowEvent::CloseRequested { api, .. } = event {
        let app = window.app_handle();
        let to_tray = app
            .try_state::<TrayState>()
            .is_some_and(|s| s.tray.is_some() && s.settings().close_to_tray);
        if to_tray {
            api.prevent_close();
            hide_main(app);
        }
    }
}

/// Show, unminimize and focus the main window.
pub fn show_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    if let Err(e) = window.show() {
        tracing::warn!("could not show the window: {e}");
    }
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    let _ = window.set_focus();
    visibility_changed(app, true);
}

fn hide_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    if let Err(e) = window.hide() {
        tracing::warn!("could not hide the window: {e}");
    }
    visibility_changed(app, false);
}

fn visibility_changed(app: &AppHandle, visible: bool) {
    if let Err(e) = app.emit(VISIBLE_EVENT, visible) {
        tracing::warn!("could not emit {VISIBLE_EVENT}: {e}");
    }
    if let Some(state) = app.try_state::<TrayState>() {
        let _ = state.toggle.set_text(toggle_label(visible));
    }
}

fn toggle_main(app: &AppHandle) {
    let shown = app
        .get_webview_window(MAIN)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
    if shown {
        hide_main(app);
    } else {
        show_main(app);
    }
}

/// Whether alerts should raise a desktop notification ("Notify me of alerts").
pub fn notify_enabled(app: &AppHandle) -> bool {
    app.try_state::<TrayState>()
        .is_none_or(|s| s.settings().notify_alerts)
}

/// Set the disabled status line in the tray menu, e.g. "Tailing 2 files".
pub fn set_status(app: &AppHandle, text: &str) {
    if let Some(state) = app.try_state::<TrayState>() {
        if let Err(e) = state.status.set_text(text) {
            tracing::warn!("could not set the tray status: {e}");
        }
    }
}

fn on_tray_event(tray: &TrayIcon, event: TrayIconEvent) {
    // On Linux this needs the ksni (StatusNotifierItem) backend, enabled in
    // Cargo.toml: AppIndicator never reported clicks. ksni sends a left click
    // as one Left/Up event.
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        toggle_main(tray.app_handle());
    }
}

/// Event sent to the page when a startup switch changes (from the tray menu
/// or the Settings page), so the page's switches follow the menu's.
const STARTUP_EVENT: &str = "startup-settings";

/// The tray menu's switches, for the Settings page's Startup section.
#[derive(Debug, Clone, Serialize)]
pub struct StartupSettings {
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub notify_alerts: bool,
    pub launch_at_login: bool,
    /// The desktop has a system tray. Without one, Close to tray and Start
    /// minimized do nothing (the window can't hide where it can't come back).
    pub tray: bool,
}

fn startup_now(app: &AppHandle) -> StartupSettings {
    let state = app.try_state::<TrayState>();
    let tray = state.as_ref().is_some_and(|t| t.tray.is_some());
    let s = state.map(|t| t.settings()).unwrap_or_default();
    let launch_at_login = autolaunch(app)
        .and_then(|m| m.is_enabled().ok())
        .unwrap_or(false);
    StartupSettings {
        close_to_tray: s.close_to_tray,
        start_minimized: s.start_minimized,
        notify_alerts: s.notify_alerts,
        launch_at_login,
        tray,
    }
}

fn startup_changed(app: &AppHandle) {
    let _ = app.emit(STARTUP_EVENT, startup_now(app));
}

/// The four switches as they are now (launch at login: the OS's own registration).
#[tauri::command]
pub fn startup_settings(app: AppHandle) -> StartupSettings {
    startup_now(&app)
}

/// Change one switch from the Settings page; the tray menu's check mark follows.
#[tauri::command]
pub fn set_startup_setting(
    app: AppHandle,
    key: String,
    value: bool,
) -> Result<StartupSettings, String> {
    let state = app
        .try_state::<TrayState>()
        .ok_or("the tray isn't set up")?;
    match key.as_str() {
        "close_to_tray" => {
            state.update(|s| s.close_to_tray = value);
            let _ = state.close_to_tray.set_checked(value);
        }
        "start_minimized" => {
            state.update(|s| s.start_minimized = value);
            let _ = state.start_minimized.set_checked(value);
        }
        "notify_alerts" => {
            state.update(|s| s.notify_alerts = value);
            let _ = state.notify.set_checked(value);
        }
        "launch_at_login" => {
            let manager = autolaunch(&app).ok_or("the autostart plugin is not registered")?;
            let result = if value {
                manager.enable()
            } else {
                manager.disable()
            };
            result.map_err(|e| format!("could not change launch at login: {e}"))?;
            let _ = state.launch_at_login.set_checked(value);
        }
        other => return Err(format!("unknown setting {other}")),
    }
    startup_changed(&app);
    Ok(startup_now(&app))
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    // The menu flips a check item's mark itself before this runs, so the
    // mark already shows the new value.
    match event.id().as_ref() {
        ID_TOGGLE => toggle_main(app),
        ID_QUIT => app.exit(0),
        ID_CLOSE_TO_TRAY => {
            let on = state.close_to_tray.is_checked().unwrap_or(true);
            state.update(|s| s.close_to_tray = on);
        }
        ID_NOTIFY => {
            let on = state.notify.is_checked().unwrap_or(true);
            state.update(|s| s.notify_alerts = on);
        }
        ID_START_MINIMIZED => {
            let on = state.start_minimized.is_checked().unwrap_or(false);
            state.update(|s| s.start_minimized = on);
        }
        ID_LAUNCH_AT_LOGIN => {
            let on = state.launch_at_login.is_checked().unwrap_or(false);
            let result = match autolaunch(app) {
                Some(manager) if on => manager.enable().map_err(|e| e.to_string()),
                Some(manager) => manager.disable().map_err(|e| e.to_string()),
                None => Err("the autostart plugin is not registered".to_string()),
            };
            if let Err(e) = result {
                tracing::warn!("could not change launch at login: {e}");
                let _ = state.launch_at_login.set_checked(!on);
            }
        }
        _ => return,
    }
    // A switch changed from the menu: the Settings page follows.
    if ![ID_TOGGLE, ID_QUIT].contains(&event.id().as_ref()) {
        startup_changed(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh empty directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "oxitailr-tray-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = TempDir::new();
        let s = load_settings_from(&dir.0.join(SETTINGS_FILE));
        assert_eq!(s, DesktopSettings::default());
        assert!(s.close_to_tray);
        assert!(!s.start_minimized);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new();
        // A parent that does not exist yet is created.
        let path = dir.0.join("nested").join(SETTINGS_FILE);
        let s = DesktopSettings {
            close_to_tray: false,
            start_minimized: true,
            notify_alerts: true,
        };
        save_settings_to(&path, &s).unwrap();
        assert_eq!(load_settings_from(&path), s);
        // No temp file is left behind.
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from(SETTINGS_FILE)]);
    }

    #[test]
    fn save_overwrites_existing_file() {
        let dir = TempDir::new();
        let path = dir.0.join(SETTINGS_FILE);
        save_settings_to(&path, &DesktopSettings::default()).unwrap();
        let s = DesktopSettings {
            close_to_tray: false,
            start_minimized: false,
            notify_alerts: true,
        };
        save_settings_to(&path, &s).unwrap();
        assert_eq!(load_settings_from(&path), s);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let dir = TempDir::new();
        let path = dir.0.join(SETTINGS_FILE);
        std::fs::write(&path, r#"{"start_minimized": true}"#).unwrap();
        let s = load_settings_from(&path);
        assert!(s.close_to_tray);
        assert!(s.start_minimized);
    }

    #[test]
    fn corrupt_file_is_moved_aside() {
        let dir = TempDir::new();
        let path = dir.0.join(SETTINGS_FILE);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_settings_from(&path), DesktopSettings::default());
        assert!(!path.exists());
        let aside = dir.0.join(format!("{SETTINGS_FILE}.corrupt"));
        assert_eq!(std::fs::read_to_string(aside).unwrap(), "{ not json");
        // A later save works and the next load reads it.
        let s = DesktopSettings {
            close_to_tray: false,
            start_minimized: true,
            notify_alerts: true,
        };
        save_settings_to(&path, &s).unwrap();
        assert_eq!(load_settings_from(&path), s);
    }

    #[test]
    fn toggle_label_follows_visibility() {
        assert_eq!(toggle_label(true), "Hide Oxitailr");
        assert_eq!(toggle_label(false), "Show Oxitailr");
    }
}
