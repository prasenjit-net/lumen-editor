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
    edits: Vec<MenuItem>,
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
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(Some("Hide Lumen")),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &item("quit", "Quit Lumen", Code::KeyQ, false),
        ])?;
        let file = Submenu::new("File", true);
        let save = item("save", "Save", Code::KeyS, false);
        let save_as = item("save_as", "Save As…", Code::KeyS, true);
        let close = item("close", "Close Editor", Code::KeyW, false);
        file.append_items(&[
            &item("new", "New Text File", Code::KeyN, false),
            &item("open", "Open File…", Code::KeyO, false),
            &item("folder", "Open Folder…", Code::KeyO, true),
            &PredefinedMenuItem::separator(),
            &save,
            &save_as,
            &PredefinedMenuItem::separator(),
            &close,
        ])?;
        #[cfg(target_os = "windows")]
        file.append_items(&[
            &PredefinedMenuItem::separator(),
            &MenuItem::with_id("quit", "Exit", true, None),
        ])?;
        let edit = Submenu::new("Edit", true);
        let edits = vec![
            item("undo", "Undo", Code::KeyZ, false),
            item("redo", "Redo", Code::KeyZ, true),
            item("cut", "Cut", Code::KeyX, false),
            item("copy", "Copy", Code::KeyC, false),
            item("paste", "Paste", Code::KeyV, false),
            item("select_all", "Select All", Code::KeyA, false),
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
        let view = Submenu::new("View", true);
        view.append_items(&[
            &item("palette", "Command Palette…", Code::KeyP, true),
            &item("sidebar", "Toggle Sidebar", Code::KeyB, false),
        ])?;
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
            "welcome",
            "Welcome to Lumen",
            true,
            None,
        ))?;
        #[cfg(target_os = "macos")]
        menu.append_items(&[&app, &file, &edit, &view, &window, &help])?;
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
            menu.append_items(&[&file, &edit, &view, &window, &help])?;
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
            edits,
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
            unsafe {
                self._menu
                    .init_for_hwnd_with_theme(handle.hwnd.get(), muda::MenuTheme::Dark)?;
            }
        }
        Ok(())
    }

    pub fn actions(&self) -> Vec<String> {
        self.events.try_iter().map(|event| event.id.0).collect()
    }

    pub fn set_document_open(&self, open: bool) {
        for item in [&self.save, &self.save_as, &self.close] {
            item.set_enabled(open);
        }
        for item in &self.edits {
            item.set_enabled(open);
        }
    }
}

fn item(id: &str, title: &str, code: Code, shift: bool) -> MenuItem {
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
    MenuItem::with_id(id, title, true, Some(Accelerator::new(modifiers, code)))
}
