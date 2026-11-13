use gpui_kit::{App, Window};

#[derive(Clone, Copy)]
pub enum Command {
    Show,
    Hide,
    Toggle,
    Quit,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub struct AppTray {
    icon: tray_icon::TrayIcon,
    show: tray_icon::menu::MenuItem,
    hide: tray_icon::menu::MenuItem,
    toggle: tray_icon::menu::MenuItem,
    quit: tray_icon::menu::MenuItem,
    running: bool,
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
impl AppTray {
    pub fn new() -> anyhow::Result<Self> {
        use tray_icon::{
            Icon, TrayIconBuilder,
            menu::{Menu, MenuItem, PredefinedMenuItem},
        };
        let menu = Menu::new();
        let show = MenuItem::new("显示主窗口", true, None);
        let hide = MenuItem::new("收起到托盘", true, None);
        let toggle = MenuItem::new("启动监听", true, None);
        let quit = MenuItem::new("退出 Roc Request Bin", true, None);
        menu.append_items(&[
            &show,
            &hide,
            &PredefinedMenuItem::separator(),
            &toggle,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let icon = Icon::from_rgba(include_bytes!("../assets/tray.rgba").to_vec(), 32, 32)?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("Roc Request Bin · 已停止")
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()?;
        Ok(Self {
            icon,
            show,
            hide,
            toggle,
            quit,
            running: false,
        })
    }
    pub fn poll(&self) -> Vec<Command> {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent, menu::MenuEvent};
        let mut commands = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id;
            if id == self.show.id() {
                commands.push(Command::Show);
            } else if id == self.hide.id() {
                commands.push(Command::Hide);
            } else if id == self.toggle.id() {
                commands.push(Command::Toggle);
            } else if id == self.quit.id() {
                commands.push(Command::Quit);
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) && cfg!(target_os = "windows")
            {
                commands.push(Command::Show);
            }
        }
        commands
    }
    pub fn set_running(&mut self, running: bool) {
        if self.running == running {
            return;
        }
        self.running = running;
        self.toggle.set_text(if running {
            "停止监听"
        } else {
            "启动监听"
        });
        let _ = self.icon.set_tooltip(Some(if running {
            "Roc Request Bin · 监听中"
        } else {
            "Roc Request Bin · 已停止"
        }));
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub struct AppTray;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl AppTray {
    pub fn new() -> anyhow::Result<Self> {
        anyhow::bail!("Tray is unavailable on this platform")
    }
    pub fn poll(&self) -> Vec<Command> {
        vec![]
    }
    pub fn set_running(&mut self, _: bool) {}
}

pub fn hide(window: &mut Window, cx: &mut App) {
    #[cfg(target_os = "windows")]
    if let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window)
        && let raw_window_handle::RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        // The HWND belongs to this live GPUI window and is used on its UI thread.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _),
                windows::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
        }
    }
    #[cfg(target_os = "macos")]
    cx.hide();
    let _ = (window, cx);
}
pub fn show(window: &mut Window, cx: &mut App) {
    #[cfg(target_os = "windows")]
    if let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window)
        && let raw_window_handle::RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindow(
                windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _),
                windows::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
    }
    cx.activate(true);
    window.activate_window();
}
