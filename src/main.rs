use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Frame, Margin, RichText, ScrollArea, Shadow, Stroke,
    TextEdit, TextStyle, TopBottomPanel, Vec2,
};
mod icon;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native_menu;
use rfd::FileDialog;

const BG: Color32 = Color32::from_rgb(30, 30, 30);
const PANEL: Color32 = Color32::from_rgb(37, 37, 38);
const SIDEBAR: Color32 = Color32::from_rgb(51, 51, 51);
const TAB: Color32 = Color32::from_rgb(45, 45, 45);
#[cfg(not(target_os = "windows"))]
const TITLE: Color32 = Color32::from_rgb(60, 60, 60);
const STATUS: Color32 = Color32::from_rgb(0, 122, 204);
const ACTIVE: Color32 = Color32::from_rgb(4, 57, 94);
const TEXT: Color32 = Color32::from_rgb(204, 204, 204);
const MUTED: Color32 = Color32::from_rgb(157, 157, 157);
const BORDER: Color32 = Color32::from_rgb(43, 43, 43);
const ACCENT: Color32 = Color32::from_rgb(0, 122, 204);
const HOVER: Color32 = Color32::from_rgb(43, 43, 43);
const INPUT: Color32 = Color32::from_rgb(49, 49, 49);

// A two-pixel gutter exposes subtle corners without floating-card spacing.
fn surface(fill: Color32) -> Frame {
    Frame::default()
        .fill(fill)
        .corner_radius(CornerRadius::same(5))
}

fn canvas() -> Frame {
    Frame::default()
        .fill(Color32::from_gray(24))
        .inner_margin(Margin::same(2))
}

fn explorer_row(ui: &mut egui::Ui, text: String, indent: usize, selected: bool) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::ZERO,
            if selected { ACTIVE } else { HOVER },
        );
    }
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .text(
            egui::pos2(rect.left() + indent as f32, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.0),
            TEXT,
        );
    response
}

struct OpenFile {
    path: Option<PathBuf>,
    name: String,
    language: String,
    text: String,
    dirty: bool,
}

impl OpenFile {
    fn untitled(number: usize) -> Self {
        Self {
            path: None,
            name: format!("untitled-{number}.txt"),
            language: "Plain Text".into(),
            text: String::new(),
            dirty: false,
        }
    }
}

struct LumenApp {
    activity: usize,
    active_file: Option<usize>,
    files: Vec<OpenFile>,
    workspace: Option<PathBuf>,
    expanded_dirs: HashSet<PathBuf>,
    command_palette: bool,
    search: String,
    sidebar_visible: bool,
    status: String,
    next_untitled: usize,
    pending_close: Option<usize>,
    window_title: String,
    pending_quit: bool,
    allow_quit: bool,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_menu: Option<native_menu::NativeMenu>,
}

impl Default for LumenApp {
    fn default() -> Self {
        Self {
            activity: 0,
            active_file: None,
            files: Vec::new(),
            workspace: None,
            expanded_dirs: HashSet::new(),
            command_palette: false,
            search: String::new(),
            sidebar_visible: true,
            status: "Ready".into(),
            next_untitled: 1,
            pending_close: None,
            window_title: String::new(),
            pending_quit: false,
            allow_quit: false,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            native_menu: None,
        }
    }
}

impl LumenApp {
    fn request_quit(&mut self, ctx: &egui::Context) {
        if self.files.iter().any(|file| file.dirty) {
            self.pending_quit = true;
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn quit_dialog(&mut self, ctx: &egui::Context) {
        if !self.pending_quit {
            return;
        }
        egui::Window::new("Save changes before quitting?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("There are unsaved changes in your open files.");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save All").clicked() {
                        let active = self.active_file;
                        for index in 0..self.files.len() {
                            if self.files[index].dirty {
                                self.active_file = Some(index);
                                self.save_active();
                                if self.files[index].dirty {
                                    break;
                                }
                            }
                        }
                        self.active_file = active;
                        if self.files.iter().all(|file| !file.dirty) {
                            self.allow_quit = true;
                            self.pending_quit = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    if ui.button("Don't Save").clicked() {
                        self.allow_quit = true;
                        self.pending_quit = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_quit = false;
                    }
                });
            });
    }
    fn icon_button(ui: &mut egui::Ui, icon: &str, selected: bool) -> egui::Response {
        ui.add(
            egui::Button::new(RichText::new(icon).size(24.0).color(if selected {
                Color32::from_gray(215)
            } else {
                MUTED
            }))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::ZERO)
            .min_size(Vec2::splat(48.0)),
        )
    }

    fn language_for(path: &Path) -> String {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
        {
            "rs" => "Rust",
            "toml" => "TOML",
            "md" => "Markdown",
            "json" => "JSON",
            "yaml" | "yml" => "YAML",
            "js" | "ts" | "tsx" | "jsx" => "JavaScript",
            "html" => "HTML",
            "css" => "CSS",
            "py" => "Python",
            "go" => "Go",
            "c" | "h" | "cpp" | "hpp" => "C/C++",
            "sh" => "Shell Script",
            _ => "Plain Text",
        }
        .into()
    }

    fn file_icon(path: &Path) -> &'static str {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
        {
            "rs" | "go" | "c" | "h" | "cpp" | "hpp" | "js" | "ts" | "tsx" | "jsx" => {
                icon::FILE_CODE
            }
            "toml" | "json" | "yaml" | "yml" => icon::GEAR,
            "md" | "txt" => icon::FILE_TEXT,
            _ => icon::FILE,
        }
    }

    fn active(&self) -> Option<&OpenFile> {
        self.active_file.and_then(|i| self.files.get(i))
    }

    fn new_file(&mut self) {
        self.files.push(OpenFile::untitled(self.next_untitled));
        self.next_untitled += 1;
        self.active_file = Some(self.files.len() - 1);
        self.status = "New untitled file".into();
    }

    fn choose_file(&mut self) {
        if let Some(path) = FileDialog::new().set_title("Open File").pick_file() {
            self.open_file(path);
        }
    }

    fn choose_folder(&mut self) {
        if let Some(folder) = FileDialog::new().set_title("Open Folder").pick_folder() {
            self.workspace = Some(folder.clone());
            self.expanded_dirs.clear();
            self.expanded_dirs.insert(folder.clone());
            self.activity = 0;
            self.sidebar_visible = true;
            self.status = format!("Opened folder {}", display_name(&folder));
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        if let Some(index) = self
            .files
            .iter()
            .position(|file| file.path.as_deref() == Some(path.as_path()))
        {
            self.active_file = Some(index);
            return;
        }
        match fs::read_to_string(&path) {
            Ok(text) => {
                self.files.push(OpenFile {
                    name: display_name(&path),
                    language: Self::language_for(&path),
                    path: Some(path),
                    text,
                    dirty: false,
                });
                self.active_file = Some(self.files.len() - 1);
                self.status = "File opened".into();
            }
            Err(error) => self.status = format!("Could not open file: {error}"),
        }
    }

    fn save_file_at(&mut self, index: usize, path: PathBuf) {
        let Some(file) = self.files.get_mut(index) else {
            return;
        };
        match fs::write(&path, &file.text) {
            Ok(()) => {
                file.name = display_name(&path);
                file.language = Self::language_for(&path);
                file.path = Some(path);
                file.dirty = false;
                self.status = "Saved".into();
            }
            Err(error) => self.status = format!("Could not save file: {error}"),
        }
    }

    fn save_active(&mut self) {
        let Some(index) = self.active_file else {
            return;
        };
        if let Some(path) = self.files[index].path.clone() {
            self.save_file_at(index, path);
        } else {
            self.save_as_active();
        }
    }

    fn save_as_active(&mut self) {
        let Some(index) = self.active_file else {
            return;
        };
        let dialog = FileDialog::new()
            .set_title("Save As")
            .set_file_name(&self.files[index].name);
        if let Some(path) = dialog.save_file() {
            self.save_file_at(index, path);
        }
    }

    fn request_close(&mut self, index: usize) {
        if self.files.get(index).is_some_and(|file| file.dirty) {
            self.pending_close = Some(index);
        } else {
            self.close_file(index);
        }
    }

    fn close_file(&mut self, index: usize) {
        if index >= self.files.len() {
            return;
        }
        self.files.remove(index);
        self.active_file = match self.files.len() {
            0 => None,
            count => Some(index.min(count - 1)),
        };
        self.status = "Closed file".into();
    }

    fn explorer(&mut self, ui: &mut egui::Ui) {
        let Some(root) = self.workspace.clone() else {
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No folder opened").color(MUTED));
                if ui
                    .button(format!("{}  Open Folder", icon::FOLDER_OPEN))
                    .clicked()
                {
                    self.choose_folder();
                }
            });
            return;
        };
        self.explorer_entry(ui, &root, 0);
    }

    fn explorer_entry(&mut self, ui: &mut egui::Ui, path: &Path, depth: usize) {
        if path.is_dir() {
            let expanded = self.expanded_dirs.contains(path);
            let response = explorer_row(
                ui,
                format!(
                    "{}  {}  {}",
                    if expanded {
                        icon::CARET_DOWN
                    } else {
                        icon::CARET_RIGHT
                    },
                    if expanded {
                        icon::FOLDER_OPEN
                    } else {
                        icon::FOLDER
                    },
                    display_name(path)
                ),
                depth * 14 + 4,
                false,
            );
            if response.clicked() {
                if expanded {
                    self.expanded_dirs.remove(path);
                } else {
                    self.expanded_dirs.insert(path.to_path_buf());
                }
            }
            if expanded {
                for child in sorted_children(path) {
                    self.explorer_entry(ui, &child, depth + 1);
                }
            }
        } else {
            let open = self
                .active()
                .and_then(|file| file.path.as_deref())
                .is_some_and(|open_path| open_path == path);
            let response = explorer_row(
                ui,
                format!("{}  {}", Self::file_icon(path), display_name(path)),
                depth * 14 + 20,
                open,
            );
            if response.clicked() {
                self.open_file(path.to_path_buf());
            }
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        if !self.sidebar_visible {
            return;
        }
        egui::SidePanel::left("explorer")
            .default_width(240.0)
            .width_range(180.0..=600.0)
            .resizable(true)
            .show_separator_line(false)
            .frame(canvas())
            .show(ctx, |ui| {
                surface(PANEL).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.add_sized(
                            [(ui.available_width() - 114.0).max(0.0), 35.0],
                            egui::Label::new(
                                RichText::new(match self.activity {
                                    0 => "EXPLORER",
                                    1 => "SEARCH",
                                    2 => "SOURCE CONTROL",
                                    3 => "RUN AND DEBUG",
                                    _ => "EXTENSIONS",
                                })
                                .size(11.0)
                                .color(TEXT),
                            ),
                        );
                        if self.activity == 0 {
                            if ui
                                .button(icon::FILE_PLUS)
                                .on_hover_text("New file")
                                .clicked()
                            {
                                self.new_file();
                            }
                            if ui
                                .button(icon::FILE_ARROW_UP)
                                .on_hover_text("Open file")
                                .clicked()
                            {
                                self.choose_file();
                            }
                            if ui
                                .button(icon::FOLDER_OPEN)
                                .on_hover_text("Open folder")
                                .clicked()
                            {
                                self.choose_folder();
                            }
                        }
                    });
                    match self.activity {
                        0 => {
                            ScrollArea::vertical()
                                .id_salt("explorer_scroll")
                                .show(ui, |ui| self.explorer(ui));
                        }
                        1 => {
                            ui.label(RichText::new("Search").size(18.0));
                            ui.add_space(4.0);
                            ui.add(
                                TextEdit::singleline(&mut self.search)
                                    .hint_text("Search files")
                                    .desired_width(f32::INFINITY),
                            );
                        }
                        2 => {
                            ui.label(RichText::new("Source Control").size(18.0));
                            ui.label(RichText::new("Git integration is coming next.").color(MUTED));
                        }
                        3 => {
                            ui.label(RichText::new("Run and Debug").size(18.0));
                            ui.add_sized(
                                [ui.available_width(), 32.0],
                                egui::Button::new(format!("{}  Run project", icon::PLAY)),
                            );
                        }
                        _ => {
                            ui.label(RichText::new("Extensions").size(18.0));
                            ui.add(
                                TextEdit::singleline(&mut self.search)
                                    .hint_text("Search extensions")
                                    .desired_width(f32::INFINITY),
                            );
                        }
                    }
                });
            });
    }

    fn editor_tabs(&mut self, ui: &mut egui::Ui) {
        let mut select = None;
        let mut close = None;
        ScrollArea::horizontal()
            .id_salt("editor_tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (index, file) in self.files.iter().enumerate() {
                        let selected = self.active_file == Some(index);
                        Frame::default()
                            .fill(if selected { BG } else { TAB })
                            .inner_margin(Margin::symmetric(10, 6))
                            .corner_radius(CornerRadius {
                                nw: 3,
                                ne: 3,
                                sw: 0,
                                se: 0,
                            })
                            .show(ui, |ui| {
                                let start = ui.cursor().min;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let path =
                                        file.path.as_deref().unwrap_or(Path::new(&file.name));
                                    let title = format!(
                                        "{} {}{}",
                                        Self::file_icon(path),
                                        file.name,
                                        if file.dirty { " ●" } else { "" }
                                    );
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(title).color(if selected {
                                                    TEXT
                                                } else {
                                                    MUTED
                                                }),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        select = Some(index);
                                    }
                                    if ui
                                        .add(egui::Button::new(icon::X).frame(false))
                                        .on_hover_text("Close editor")
                                        .clicked()
                                    {
                                        close = Some(index);
                                    }
                                });
                                let rect = ui.min_rect();
                                if selected {
                                    ui.painter().hline(
                                        (start.x - 7.0)..=(rect.right() + 7.0),
                                        start.y - 6.0,
                                        Stroke::new(1.0_f32, ACCENT),
                                    );
                                }
                            });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button(icon::PLUS)
                            .on_hover_text("New file")
                            .clicked()
                        {
                            self.new_file();
                        }
                    });
                });
            });
        if let Some(index) = select {
            self.active_file = Some(index);
        }
        if let Some(index) = close {
            self.request_close(index);
        }
    }

    fn editor(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(canvas())
            .show(ctx, |ui| {
                surface(BG).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    self.editor_tabs(ui);
                    let Some(index) = self.active_file else {
                        ui.vertical_centered(|ui| {
                            ui.add_space(130.0);
                            ui.label(RichText::new("LUMEN").size(28.0).strong().color(MUTED));
                            ui.label(RichText::new("A focused local file editor").color(MUTED));
                            ui.add_space(20.0);
                            if ui
                                .button(format!("{}  Open File", icon::FILE_ARROW_UP))
                                .clicked()
                            {
                                self.choose_file();
                            }
                            if ui
                                .button(format!("{}  Open Folder", icon::FOLDER_OPEN))
                                .clicked()
                            {
                                self.choose_folder();
                            }
                            ui.label(
                                RichText::new("⌘O to open a file  •  ⌘N for a new file")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        });
                        return;
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        if let Some(root) = &self.workspace {
                            ui.label(RichText::new(display_name(root)).size(11.0).color(MUTED));
                            ui.label(RichText::new(icon::CARET_RIGHT).color(MUTED));
                        }
                        ui.label(
                            RichText::new(&self.files[index].name)
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                    ui.add_space(2.0);
                    let line_count = self.files[index].text.split('\n').count();
                    let content_height = ui.available_height();
                    Frame::default()
                        .fill(BG)
                        .corner_radius(CornerRadius::same(4))
                        .inner_margin(Margin::ZERO)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal_top(|ui| {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(48.0, content_height),
                                    egui::Layout::top_down(egui::Align::Max),
                                    |ui| {
                                        ScrollArea::vertical()
                                            .id_salt("line_numbers")
                                            .max_height(content_height)
                                            .show(ui, |ui| {
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                for number in 1..=line_count {
                                                    ui.label(
                                                        RichText::new(format!("{number}"))
                                                            .font(FontId::monospace(14.0))
                                                            .color(MUTED),
                                                    );
                                                }
                                            });
                                    },
                                );
                                let width = ui.available_width();
                                ui.allocate_ui_with_layout(
                                    Vec2::new(width, content_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ScrollArea::vertical()
                                            .id_salt("code_editor")
                                            .max_height(content_height)
                                            .show(ui, |ui| {
                                                if ui
                                                    .add(
                                                        TextEdit::multiline(
                                                            &mut self.files[index].text,
                                                        )
                                                        .font(FontId::monospace(14.0))
                                                        .text_color(TEXT)
                                                        .desired_width(f32::INFINITY)
                                                        .frame(false)
                                                        .code_editor(),
                                                    )
                                                    .changed()
                                                {
                                                    self.files[index].dirty = true;
                                                }
                                            });
                                    },
                                );
                            });
                        });
                });
            });
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn file_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("File", |ui| {
            if ui
                .button(format!("{}  New Text File     ⌘N", icon::FILE_PLUS))
                .clicked()
            {
                self.new_file();
                ui.close();
            }
            if ui
                .button(format!("{}  Open File…          ⌘O", icon::FILE_ARROW_UP))
                .clicked()
            {
                self.choose_file();
                ui.close();
            }
            if ui
                .button(format!("{}  Open Folder…     ⌘⇧O", icon::FOLDER_OPEN))
                .clicked()
            {
                self.choose_folder();
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled(
                    self.active_file.is_some(),
                    egui::Button::new(format!("{}  Save                 ⌘S", icon::FLOPPY_DISK)),
                )
                .clicked()
            {
                self.save_active();
                ui.close();
            }
            if ui
                .add_enabled(
                    self.active_file.is_some(),
                    egui::Button::new(format!("{}  Save As…          ⌘⇧S", icon::FLOPPY_DISK)),
                )
                .clicked()
            {
                self.save_as_active();
                ui.close();
            }
        });
    }

    fn palette(&mut self, ctx: &egui::Context) {
        if !self.command_palette {
            return;
        }
        egui::Window::new("Command Palette")
            .collapsible(false)
            .resizable(false)
            .default_width(560.0)
            .anchor(egui::Align2::CENTER_TOP, Vec2::new(0.0, 72.0))
            .frame(
                Frame::window(&ctx.style())
                    .fill(PANEL)
                    .corner_radius(CornerRadius::same(6))
                    .stroke(Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text("Type a command")
                        .desired_width(f32::INFINITY),
                );
                ui.separator();
                for (label, action) in [
                    (format!("{}  File: New Text File", icon::FILE_PLUS), 0),
                    (format!("{}  File: Open File", icon::FILE_ARROW_UP), 1),
                    (format!("{}  File: Open Folder", icon::FOLDER_OPEN), 2),
                    (format!("{}  File: Save", icon::FLOPPY_DISK), 3),
                ] {
                    if ui.selectable_label(false, label).clicked() {
                        match action {
                            0 => self.new_file(),
                            1 => self.choose_file(),
                            2 => self.choose_folder(),
                            _ => self.save_active(),
                        }
                        self.command_palette = false;
                    }
                }
            });
    }

    fn unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        let Some(index) = self.pending_close else {
            return;
        };
        let name = self
            .files
            .get(index)
            .map(|file| file.name.clone())
            .unwrap_or_default();
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("Save changes to {name}?"));
                ui.label(RichText::new("Your edits will be lost if you don't save.").color(MUTED));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save_active();
                        self.pending_close = None;
                        if self.files.get(index).is_some_and(|file| !file.dirty) {
                            self.close_file(index);
                        }
                    }
                    if ui.button("Don't Save").clicked() {
                        self.pending_close = None;
                        self.close_file(index);
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_close = None;
                    }
                });
            });
    }
}

impl eframe::App for LumenApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx.input(|input| input.viewport().close_requested())
            && !self.allow_quit
            && self.files.iter().any(|file| file.dirty)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_quit = true;
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let actions = self
                .native_menu
                .as_ref()
                .map(|menu| menu.actions())
                .unwrap_or_default();
            for action in actions {
                match action.as_str() {
                    "new" => self.new_file(),
                    "open" => self.choose_file(),
                    "folder" => self.choose_folder(),
                    "save" => self.save_active(),
                    "save_as" => self.save_as_active(),
                    "close" => {
                        if let Some(index) = self.active_file {
                            self.request_close(index);
                        }
                    }
                    "quit" => self.request_quit(ctx),
                    "palette" => self.command_palette = true,
                    "sidebar" => self.sidebar_visible = !self.sidebar_visible,
                    "welcome" => self.status = "Lumen — New: ⌘N · Open: ⌘O · Save: ⌘S".into(),
                    "copy" => ctx.input_mut(|input| input.events.push(egui::Event::Copy)),
                    "cut" => ctx.input_mut(|input| input.events.push(egui::Event::Cut)),
                    "paste" => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
                    "undo" | "redo" | "select_all" => ctx.input_mut(|input| {
                        input.events.push(egui::Event::Key {
                            key: if action == "select_all" {
                                egui::Key::A
                            } else {
                                egui::Key::Z
                            },
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers {
                                command: true,
                                mac_cmd: cfg!(target_os = "macos"),
                                ctrl: cfg!(target_os = "windows"),
                                shift: action == "redo",
                                ..Default::default()
                            },
                        });
                    }),
                    _ => {}
                }
            }
            if let Some(menu) = &self.native_menu {
                menu.set_document_open(self.active_file.is_some());
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::P)) {
                self.command_palette = true;
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::B)) {
                self.sidebar_visible = !self.sidebar_visible;
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::N)) {
                self.new_file();
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.choose_folder();
                } else {
                    self.choose_file();
                }
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.save_as_active();
                } else {
                    self.save_active();
                }
            }
            if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::W)) {
                if let Some(index) = self.active_file {
                    self.request_close(index);
                }
            }
        }

        let title = self
            .active()
            .map(|file| {
                format!(
                    "{}{} — Lumen",
                    file.name,
                    if file.dirty { " ●" } else { "" }
                )
            })
            .unwrap_or_else(|| "Lumen".into());
        if self.window_title != title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
        #[cfg(target_os = "macos")]
        TopBottomPanel::top("mac_titlebar")
            .exact_height(32.0)
            .frame(Frame::default().fill(TITLE))
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 32.0),
                    egui::Sense::hover(),
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &self.window_title,
                    FontId::proportional(13.0),
                    TEXT,
                );
                // Leave the leftmost 80px to AppKit's real traffic-light buttons.
                let drag_rect = egui::Rect::from_min_max(rect.min + Vec2::new(80.0, 0.0), rect.max);
                let response = ui.interact(
                    drag_rect,
                    ui.id().with("window_drag"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    let maximized = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
            });
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        TopBottomPanel::top("titlebar")
            .exact_height(35.0)
            .frame(
                Frame::default()
                    .fill(TITLE)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ctx, |ui| {
                Frame::default().fill(TITLE).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{}  LUMEN", icon::BRACKETS_CURLY))
                                .strong()
                                .color(MUTED),
                        );
                        ui.add_space(12.0);
                        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                        {
                            self.file_menu(ui);
                            for item in
                                ["Edit", "Selection", "View", "Go", "Run", "Terminal", "Help"]
                            {
                                ui.label(RichText::new(item).size(12.0).color(TEXT));
                                ui.add_space(2.0);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let title = self
                                .active()
                                .map(|file| {
                                    format!(
                                        "{}{} — Lumen",
                                        file.name,
                                        if file.dirty { " ●" } else { "" }
                                    )
                                })
                                .unwrap_or_else(|| "Lumen".into());
                            ui.label(RichText::new(title).color(MUTED).size(12.0));
                        });
                    });
                });
            });
        TopBottomPanel::bottom("status")
            .exact_height(22.0)
            .frame(
                Frame::default()
                    .fill(STATUS)
                    .inner_margin(Margin::symmetric(8, 0)),
            )
            .show(ctx, |ui| {
                Frame::default().fill(STATUS).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&self.status).size(11.0).color(Color32::WHITE));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{}   UTF-8   LF",
                                    self.active()
                                        .map(|file| file.language.as_str())
                                        .unwrap_or("Plain Text")
                                ))
                                .size(11.0)
                                .color(Color32::WHITE),
                            );
                        });
                    });
                });
            });
        egui::SidePanel::left("activity")
            .exact_width(52.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(canvas())
            .show(ctx, |ui| {
                surface(SIDEBAR).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    ui.vertical_centered(|ui| {
                        for (i, glyph) in [
                            icon::FILES,
                            icon::MAGNIFYING_GLASS,
                            icon::GIT_BRANCH,
                            icon::DEBUG,
                            icon::PUZZLE_PIECE,
                        ]
                        .iter()
                        .enumerate()
                        {
                            let response = Self::icon_button(ui, glyph, self.activity == i);
                            if self.activity == i {
                                ui.painter().vline(
                                    response.rect.left(),
                                    response.rect.y_range(),
                                    Stroke::new(2.0_f32, ACCENT),
                                );
                            }
                            if response.clicked() {
                                self.activity = i;
                                self.sidebar_visible = true;
                            }
                        }
                        ui.add_space((ui.available_height() - 50.0).max(0.0));
                        Self::icon_button(ui, icon::GEAR, false);
                    });
                });
            });
        self.sidebar(ctx);
        self.editor(ctx);
        self.palette(ctx);
        self.unsaved_changes_dialog(ctx);
        self.quit_dialog(ctx);
    }
}

fn sorted_children(path: &Path) -> Vec<PathBuf> {
    let mut children: Vec<_> = fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some(".git") | Some("target") | Some(".DS_Store")
            )
        })
        .collect();
    children.sort_by(|left, right| {
        right.is_dir().cmp(&left.is_dir()).then_with(|| {
            display_name(left)
                .to_lowercase()
                .cmp(&display_name(right).to_lowercase())
        })
    });
    children
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Untitled")
        .into()
}

fn configure_ui(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    style.spacing.item_spacing = Vec2::new(6.0, 0.0);
    style.spacing.button_padding = Vec2::new(6.0, 2.0);
    style.spacing.interact_size = Vec2::new(20.0, 22.0);
    style.interaction.resize_grab_radius_side = 5.0;
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(12.5));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(18.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.5));

    let visuals = &mut style.visuals;
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.window_corner_radius = CornerRadius::same(6);
    visuals.menu_corner_radius = CornerRadius::same(5);
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow::NONE;
    visuals.selection.bg_fill = ACTIVE;
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.text_edit_bg_color = Some(INPUT);
    visuals.extreme_bg_color = BG;
    visuals.weak_text_color = Some(MUTED);
    visuals.hyperlink_color = ACCENT;
    // Text fields use noninteractive visuals when idle. Define all five states
    // so buttons, inputs, menus and selections share the same palette and radius.
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = INPUT;
        widget.weak_bg_fill = Color32::TRANSPARENT;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widget.corner_radius = CornerRadius::same(3);
        widget.expansion = 0.0;
    }
    visuals.widgets.noninteractive.fg_stroke.color = MUTED;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, Color32::from_gray(60));
    visuals.widgets.hovered.bg_fill = HOVER;
    visuals.widgets.hovered.weak_bg_fill = HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.active.bg_fill = ACTIVE;
    visuals.widgets.active.weak_bg_fill = ACTIVE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.open.bg_fill = ACTIVE;
    visuals.widgets.open.weak_bg_fill = ACTIVE;
    visuals.widgets.open.bg_stroke.color = ACCENT;
    ctx.set_style(style);
}

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1280.0, 760.0])
        .with_min_inner_size([900.0, 560.0]);
    #[cfg(target_os = "macos")]
    let viewport = viewport
        .with_fullsize_content_view(true)
        .with_titlebar_shown(false)
        .with_title_shown(false)
        .with_titlebar_buttons_shown(true);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Lumen",
        options,
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "codicon".into(),
                egui::FontData::from_static(include_bytes!("../assets/codicon.ttf")).into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("codicon".into());
            // Use platform fonts when available, retaining embedded fallbacks.
            #[cfg(not(target_os = "windows"))]
            let platform_fonts = [
                (
                    "system-ui",
                    "/System/Library/Fonts/Supplemental/Arial.ttf",
                    egui::FontFamily::Proportional,
                ),
                (
                    "editor-mono",
                    "/System/Library/Fonts/Menlo.ttc",
                    egui::FontFamily::Monospace,
                ),
            ];
            #[cfg(target_os = "windows")]
            let platform_fonts = [
                (
                    "system-ui",
                    r"C:\Windows\Fonts\segoeui.ttf",
                    egui::FontFamily::Proportional,
                ),
                (
                    "editor-mono",
                    r"C:\Windows\Fonts\consola.ttf",
                    egui::FontFamily::Monospace,
                ),
            ];
            for (name, path, family) in platform_fonts {
                if let Ok(bytes) = fs::read(path) {
                    fonts
                        .font_data
                        .insert(name.into(), egui::FontData::from_owned(bytes).into());
                    fonts
                        .families
                        .entry(family)
                        .or_default()
                        .insert(0, name.into());
                }
            }
            cc.egui_ctx.set_fonts(fonts);
            configure_ui(&cc.egui_ctx);
            let mut app = LumenApp::default();
            for argument in std::env::args_os().skip(1) {
                let path = PathBuf::from(argument);
                if path.is_dir() {
                    app.workspace = Some(path.clone());
                    app.expanded_dirs.insert(path);
                } else {
                    app.open_file(path);
                }
            }
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                let menu = native_menu::NativeMenu::new(&cc.egui_ctx)?;
                #[cfg(target_os = "windows")]
                menu.attach(cc)?;
                menu.set_document_open(app.active_file.is_some());
                app.native_menu = Some(menu);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sidebar_frame(app: &mut LumenApp, ctx: &egui::Context, events: Vec<egui::Event>) -> f32 {
        let mut width = 0.0;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1000.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                app.sidebar(ctx);
                width = ctx.available_rect().left();
            },
        );
        width
    }

    #[test]
    fn explorer_sash_resizes_both_ways_and_retains_width() {
        let ctx = egui::Context::default();
        configure_ui(&ctx);
        let mut app = LumenApp::default();
        let initial = sidebar_frame(&mut app, &ctx, vec![]);
        let edge = egui::pos2(initial - 1.0, 200.0);
        sidebar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
        sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: edge,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let wider = sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(edge + Vec2::new(100.0, 0.0))],
        );
        assert!(
            wider > initial + 70.0,
            "drag should expand sidebar: {initial} -> {wider}"
        );
        let release = edge + Vec2::new(100.0, 0.0);
        sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: release,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let retained = sidebar_frame(&mut app, &ctx, vec![]);
        assert!(
            (retained - wider).abs() < 2.0,
            "width should persist after release: {wider} -> {retained}"
        );

        let edge = egui::pos2(retained - 1.0, 200.0);
        sidebar_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(edge)]);
        sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: edge,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let narrower = sidebar_frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(edge - Vec2::new(100.0, 0.0))],
        );
        assert!(
            narrower < wider - 70.0,
            "drag should shrink sidebar: {wider} -> {narrower}"
        );
    }
}
