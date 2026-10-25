use crate::app::{command::Command, settings::ThemeChoice};
use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};

pub struct NativeMenu {
    _menu: Menu,
    events: Receiver<MenuEvent>,
    save: MenuItem,
    save_as: MenuItem,
    close: MenuItem,
    close_folder: MenuItem,
    edits: Vec<MenuItem>,
    find: Vec<MenuItem>,
}

impl NativeMenu {
    pub fn new(ctx: &egui::Context) -> muda::Result<Self> {
        let menu = Menu::new();
        #[cfg(target_os = "macos")]
        let app = Submenu::new("Lumen", true);
        #[cfg(target_os = "macos")]
        app.append_items(&[
            &PredefinedMenuItem::about(
                Some("About Lumen"),
                Some(AboutMetadata {
                    name: Some("Lumen".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &item(Command::Settings, "Settings…", Code::Comma, false),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some("Hide Lumen")),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &item(Command::Quit, "Quit Lumen", Code::KeyQ, false),
        ])?;
        let file = Submenu::new("File", true);
        let save = item(Command::Save, "Save", Code::KeyS, false);
        let save_as = item(Command::SaveAs, "Save As…", Code::KeyS, true);
        let close = item(Command::CloseEditor, "Close Document", Code::KeyW, false);
        let close_folder = plain_item(Command::CloseFolder, "Close Folder");
        file.append_items(&[
            &item(Command::NewUntitled, "New Text File", Code::KeyN, false),
            &item(Command::OpenFile, "Open File…", Code::KeyO, false),
            &item(Command::OpenFolder, "Open Folder…", Code::KeyO, true),
            &plain_item(Command::ShowRecentFolders, "Open Recent Folder…"),
            &close_folder,
            &PredefinedMenuItem::separator(),
            &save,
            &save_as,
            &PredefinedMenuItem::separator(),
            &close,
        ])?;
        #[cfg(target_os = "windows")]
        file.append_items(&[
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id(
                Command::Quit.menu_id().expect("native command"),
                "Exit",
                true,
                None,
            ),
        ])?;
        let edit = Submenu::new("Edit", true);
        let edits = vec![
            item(Command::Undo, "Undo", Code::KeyZ, false),
            item(Command::Redo, "Redo", Code::KeyZ, true),
            item(Command::Cut, "Cut", Code::KeyX, false),
            item(Command::Copy, "Copy", Code::KeyC, false),
            item(Command::Paste, "Paste", Code::KeyV, false),
            item(Command::SelectAll, "Select All", Code::KeyA, false),
        ];
        edit.append_items(&[
            &edits[0],
            &edits[1],
            &PredefinedMenuItem::separator(),
            &edits[2],
            &edits[3],
            &edits[4],
            &PredefinedMenuItem::separator(),
            &edits[5],
        ])?;
        let find = vec![
            item(Command::Find, "Find…", Code::KeyF, false),
            if cfg!(target_os = "macos") {
                MenuItem::with_id(
                    Command::Replace.menu_id().expect("native command"),
                    "Replace…",
                    true,
                    Some(Accelerator::new(
                        Modifiers::META | Modifiers::ALT,
                        Code::KeyF,
                    )),
                )
            } else {
                item(Command::Replace, "Replace…", Code::KeyH, false)
            },
        ];
        edit.append_items(&[&PredefinedMenuItem::separator(), &find[0], &find[1]])?;
        #[cfg(target_os = "windows")]
        edit.append(&item(Command::Settings, "Settings…", Code::Comma, false))?;
        let view = Submenu::new("View", true);
        view.append_items(&[
            &item(Command::QuickOpen, "Go to File…", Code::KeyP, false),
            &item(Command::Palette, "Command Palette…", Code::KeyP, true),
            &item(Command::GoToLine, "Go to Line/Column…", Code::KeyG, false),
            &item(Command::ToggleSidebar, "Toggle Sidebar", Code::KeyB, false),
            &item(
                Command::ShowWorkspaceSearch,
                "Find in Files…",
                Code::KeyF,
                true,
            ),
            &PredefinedMenuItem::separator(),
            // Their shortcuts (Ctrl+Tab, Ctrl+Shift+Tab, Cmd+Shift+T) are handled
            // in the event loop, so the items carry no accelerator of their own.
            &plain_item(Command::NextTab, "Next Tab"),
            &plain_item(Command::PreviousTab, "Previous Tab"),
            &plain_item(Command::ReopenClosedTab, "Reopen Closed Tab"),
            // Cmd/Ctrl+Shift+V is handled in the event loop, like the tab shortcuts.
            &plain_item(Command::OpenActivePreview, "Open Preview"),
            &plain_item(Command::OpenActiveVisualEditor, "Open Visual Editor"),
        ])?;
        let theme = Submenu::new("Color Theme", true);
        for choice in ThemeChoice::ALL {
            theme.append(&plain_item(Command::SetColorTheme(choice), choice.label()))?;
        }
        view.append_items(&[&PredefinedMenuItem::separator(), &theme])?;
        // Keep the terminal discoverable from the native menu on macOS and
        // Windows as well as from the status bar, mirroring VS Code's panel
        // toggle shortcut.
        let terminal = Submenu::new("Terminal", true);
        terminal.append(&MenuItem::with_id(
            Command::ToggleTerminal.menu_id().expect("native command"),
            "Toggle Terminal",
            true,
            Some(Accelerator::new(Modifiers::CONTROL, Code::Backquote)),
        ))?;
        terminal.append(&MenuItem::with_id(
            Command::ToggleOutput.menu_id().expect("native command"),
            "Toggle Output",
            true,
            Some(Accelerator::new(
                if cfg!(target_os = "macos") {
                    Modifiers::META | Modifiers::SHIFT
                } else {
                    Modifiers::CONTROL | Modifiers::SHIFT
                },
                Code::KeyU,
            )),
        ))?;
        let window = Submenu::new("Window", true);
        #[cfg(target_os = "macos")]
        window.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::zoom(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ])?;
        #[cfg(target_os = "windows")]
        window.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
        ])?;
        let help = Submenu::new("Help", true);
        help.append(&MenuItem::with_id(
            Command::Welcome.menu_id().expect("native command"),
            "Welcome to Lumen",
            true,
            None,
        ))?;
        #[cfg(target_os = "macos")]
        menu.append_items(&[&app, &file, &edit, &view, &terminal, &window, &help])?;
        #[cfg(target_os = "windows")]
        {
            help.append(&PredefinedMenuItem::about(
                Some("About Lumen"),
                Some(AboutMetadata {
                    name: Some("Lumen".into()),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                    ..Default::default()
                }),
            ))?;
            menu.append_items(&[&file, &edit, &view, &terminal, &window, &help])?;
        }
        #[cfg(target_os = "macos")]
        window.set_as_windows_menu_for_nsapp();

        let (sender, events) = mpsc::channel();
        let ctx = ctx.clone();
        // A menu action must wake eframe even when the editor is idle.
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = sender.send(event);
            ctx.request_repaint();
        }));
        #[cfg(target_os = "macos")]
        menu.init_for_nsapp();
        Ok(Self {
            _menu: menu,
            events,
            save,
            save_as,
            close,
            close_folder,
            edits,
            find,
        })
    }

    #[cfg(target_os = "windows")]
    pub fn attach(
        &self,
        cc: &eframe::CreationContext<'_>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let RawWindowHandle::Win32(handle) = cc.window_handle()?.as_raw() {
            // The handle is provided by the live eframe window on its UI thread.
            let hwnd = handle.hwnd.get();
            unsafe {
                let before = client_height(hwnd);
                self._menu
                    .init_for_hwnd_with_theme(hwnd, muda::MenuTheme::Dark)?;
                grow_by(hwnd, before - client_height(hwnd));
            }
        }
        Ok(())
    }

    pub fn actions(&self) -> Vec<Command> {
        self.events
            .try_iter()
            .filter_map(|event| Command::from_menu_id(&event.id.0))
            .collect()
    }

    pub fn set_context(
        &self,
        open: bool,
        text_input: bool,
        explorer: bool,
        settings: bool,
        pending: bool,
        folder_open: bool,
    ) {
        for item in &self.find {
            item.set_enabled(open && !settings);
        }
        self.save.set_enabled(if settings { pending } else { open });
        self.save_as.set_enabled(open && !settings);
        self.close.set_enabled(open || settings);
        self.close_folder.set_enabled(folder_open);
        for item in &self.edits {
            let clipboard = matches!(
                Command::from_menu_id(&item.id().0),
                Some(Command::Copy | Command::Cut | Command::Paste)
            );
            item.set_enabled(text_input || (clipboard && explorer));
        }
    }
}

/// Height of the window's client area in physical pixels.
///
/// # Safety
/// `hwnd` must be a live window handle owned by the calling thread.
#[cfg(target_os = "windows")]
unsafe fn client_height(hwnd: isize) -> i32 {
    use windows_sys::Win32::{Foundation::RECT, UI::WindowsAndMessaging::GetClientRect};
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe { GetClientRect(hwnd as _, &mut rect) };
    rect.bottom - rect.top
}

/// Make the window `extra` physical pixels taller, keeping its position.
///
/// Attaching the menu bar takes its height out of the client area. Giving it
/// back before the first frame keeps the requested client size, so the first
/// frame is laid out and presented at the right size instead of showing a dead
/// strip until a later resize reconfigures the surface.
///
/// # Safety
/// `hwnd` must be a live window handle owned by the calling thread.
#[cfg(target_os = "windows")]
unsafe fn grow_by(hwnd: isize, extra: i32) {
    use windows_sys::Win32::{
        Foundation::RECT,
        UI::WindowsAndMessaging::{
            GetWindowRect, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos,
        },
    };
    if extra <= 0 {
        return;
    }
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetWindowRect(hwnd as _, &mut rect);
        SetWindowPos(
            hwnd as _,
            std::ptr::null_mut(),
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top + extra,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn item(command: Command, title: &str, code: Code, shift: bool) -> MenuItem {
    let primary = if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CONTROL
    };
    let modifiers = if shift {
        primary | Modifiers::SHIFT
    } else {
        primary
    };
    MenuItem::with_id(
        command.menu_id().expect("native command"),
        title,
        true,
        Some(Accelerator::new(modifiers, code)),
    )
}

fn plain_item(command: Command, title: &str) -> MenuItem {
    MenuItem::with_id(
        command.menu_id().expect("native command"),
        title,
        true,
        None,
    )
}
