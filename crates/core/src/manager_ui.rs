//! Platform-independent Project Manager UI (`ManagerState`).
//!
//! Renders the sidebar, search bar, board grid (with PNG thumbnails via
//! `Style::Image`), context menus, and modal dialogs into a [`Frame`].

use std::rc::Rc;

use kurbo::{Affine, BezPath, Point, Rect, RoundedRect, Shape, Size};

use crate::draw::{Color, DrawOp, Frame, Item, Style};
use crate::editor::Key;
use crate::text;
use crate::workspace::{Board, Project, Workspace, PROJECT_COLORS, RECENT_LIMIT};

const UI_FONT: i64 = 2; // Liberation Sans
const SIDEBAR_W: f64 = 248.0;
const TOPBAR_H: f64 = 68.0;
const PAD: f64 = 20.0;
const CARD_MIN_W: f64 = 230.0;
const CARD_H: f64 = 204.0;
const THUMB_H: f64 = 132.0;
const CARD_GAP: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Self::Dark)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selection {
    Recent,
    Project(String),
}

/// Actions emitted by [`ManagerState`] for the platform shell to execute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManagerAction {
    OpenBoard(String),
    CreateProject(String),
    RenameProject { project_id: String, name: String },
    RecolorProject { project_id: String, color: String },
    DeleteProject(String),
    CreateBoard { project_id: String, name: String },
    RenameBoard { board_id: String, name: String },
    DuplicateBoard(String),
    MoveBoard { board_id: String, target_project_id: String },
    DeleteBoard(String),
    ExportBoard(String),
    ImportBoards(String),
    OpenWorkspaceFolder,
    ToggleTheme(ThemeMode),
}

#[derive(Clone, Debug)]
enum Dialog {
    Prompt {
        title: String,
        value: String,
        confirm_label: String,
        kind: PromptKind,
    },
    ChooseColor {
        project_id: String,
        current: String,
    },
    ChooseProject {
        board_id: String,
        current_project_id: String,
    },
    Confirm {
        title: String,
        message: String,
        confirm_label: String,
        kind: ConfirmKind,
    },
}

#[derive(Clone, Debug)]
enum PromptKind {
    NewProject,
    RenameProject(String),
    NewBoard(String),
    RenameBoard(String),
}

#[derive(Clone, Debug)]
enum ConfirmKind {
    DeleteProject(String),
    DeleteBoard(String),
}

#[derive(Clone, Debug)]
enum HitTarget {
    SelectRecent,
    SelectProject(String),
    NewProject,
    RenameProject(String),
    RecolorProject(String),
    DeleteProject(String),
    OpenWorkspaceFolder,
    ToggleTheme,
    SearchBox,
    ClearSearch,
    NewBoard,
    ImportBoards,
    OpenBoard(String),
    CardMenuBtn(String),
    MenuAction(MenuChoice),
    DialogBackdrop,
    DialogInput,
    DialogConfirm,
    DialogCancel,
    DialogColor(String),
    DialogPickProject(String),
}

#[derive(Clone, Debug)]
enum MenuChoice {
    Open(String),
    Rename(String),
    Duplicate(String),
    Move(String),
    Export(String),
    Delete(String),
}

struct HitRegion {
    rect: Rect,
    target: HitTarget,
}

struct Palette {
    bg: Color,
    sidebar: Color,
    surface: Color,
    surface_alt: Color,
    border: Color,
    text: Color,
    muted: Color,
    accent: Color,
    accent_text: Color,
    danger: Color,
    hover: Color,
    selected: Color,
}

impl Palette {
    fn for_theme(theme: ThemeMode) -> Self {
        match theme {
            ThemeMode::Light => Self {
                bg: Color::rgba(246, 245, 241, 255),
                sidebar: Color::rgba(239, 237, 231, 255),
                surface: Color::rgba(255, 255, 255, 255),
                surface_alt: Color::rgba(243, 242, 238, 255),
                border: Color::rgba(218, 215, 206, 255),
                text: Color::rgba(30, 30, 36, 255),
                muted: Color::rgba(108, 108, 118, 255),
                accent: Color::rgba(105, 101, 219, 255),
                accent_text: Color::WHITE,
                danger: Color::rgba(210, 56, 56, 255),
                hover: Color::rgba(0, 0, 0, 14),
                selected: Color::rgba(105, 101, 219, 28),
            },
            ThemeMode::Dark => Self {
                bg: Color::rgba(18, 18, 20, 255),
                sidebar: Color::rgba(25, 25, 29, 255),
                surface: Color::rgba(34, 34, 40, 255),
                surface_alt: Color::rgba(42, 42, 49, 255),
                border: Color::rgba(58, 58, 68, 255),
                text: Color::rgba(236, 236, 241, 255),
                muted: Color::rgba(154, 154, 166, 255),
                accent: Color::rgba(124, 120, 236, 255),
                accent_text: Color::WHITE,
                danger: Color::rgba(235, 87, 87, 255),
                hover: Color::rgba(255, 255, 255, 16),
                selected: Color::rgba(124, 120, 236, 36),
            },
        }
    }
}

pub struct ManagerState {
    pub workspace: Workspace,
    pub selection: Selection,
    pub theme: ThemeMode,
    pub query: String,
    pub search_focused: bool,
    pub scroll_y: f64,
    pub size: Size,
    pub now_ms: u64,
    pub notice: Option<String>,
    open_menu_board: Option<String>,
    menu_anchor: Point,
    dialog: Option<Dialog>,
    cursor: Point,
    hits: Vec<HitRegion>,
    content_height: f64,
}

impl ManagerState {
    pub fn new(workspace: Workspace, size: Size, theme: ThemeMode, now_ms: u64) -> Self {
        Self {
            workspace,
            selection: Selection::Recent,
            theme,
            query: String::new(),
            search_focused: false,
            scroll_y: 0.0,
            size,
            now_ms,
            notice: None,
            open_menu_board: None,
            menu_anchor: Point::ZERO,
            dialog: None,
            cursor: Point::ZERO,
            hits: Vec::new(),
            content_height: 0.0,
        }
    }

    pub fn set_workspace(&mut self, workspace: Workspace) {
        self.workspace = workspace;
        if let Selection::Project(id) = &self.selection {
            if self.workspace.find_project(id).is_none() {
                self.selection = Selection::Recent;
            }
        }
        self.open_menu_board = None;
    }

    pub fn resize(&mut self, size: Size) {
        self.size = size;
        self.clamp_scroll();
    }

    pub fn pointer_move(&mut self, pos: Point) {
        self.cursor = pos;
    }

    pub fn scroll(&mut self, dy: f64) {
        if self.dialog.is_some() {
            return;
        }
        self.scroll_y -= dy;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let view_h = (self.size.height - TOPBAR_H).max(0.0);
        let max_scroll = (self.content_height - view_h).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
    }

    pub fn has_modal_or_focus(&self) -> bool {
        self.dialog.is_some() || self.search_focused
    }

    pub fn text_input(&mut self, text: &str) {
        if let Some(Dialog::Prompt { value, .. }) = &mut self.dialog {
            for ch in text.chars() {
                if !ch.is_control() && value.chars().count() < 120 {
                    value.push(ch);
                }
            }
            return;
        }
        if self.search_focused {
            for ch in text.chars() {
                if !ch.is_control() && self.query.chars().count() < 80 {
                    self.query.push(ch);
                    self.scroll_y = 0.0;
                }
            }
        }
    }

    pub fn key_down(&mut self, key: Key) -> Option<ManagerAction> {
        if let Some(dialog) = &mut self.dialog {
            match key {
                Key::Escape => {
                    self.dialog = None;
                    return None;
                }
                Key::Backspace => {
                    if let Dialog::Prompt { value, .. } = dialog {
                        value.pop();
                    }
                    return None;
                }
                Key::Enter => return self.confirm_dialog(),
                _ => return None,
            }
        }

        if self.search_focused {
            match key {
                Key::Escape => {
                    self.query.clear();
                    self.search_focused = false;
                    self.scroll_y = 0.0;
                }
                Key::Backspace => {
                    self.query.pop();
                    self.scroll_y = 0.0;
                }
                Key::Enter => {
                    self.search_focused = false;
                }
                _ => {}
            }
            return None;
        }

        if key == Key::Escape && self.open_menu_board.is_some() {
            self.open_menu_board = None;
        }
        None
    }

    pub fn click(&mut self, pos: Point) -> Option<ManagerAction> {
        self.cursor = pos;
        let hit = self.hits.iter().rev().find(|r| r.rect.contains(pos)).map(|r| r.target.clone());
        let Some(target) = hit else {
            self.open_menu_board = None;
            self.search_focused = false;
            return None;
        };

        self.search_focused = matches!(target, HitTarget::SearchBox);

        match target {
            HitTarget::SelectRecent => {
                self.selection = Selection::Recent;
                self.scroll_y = 0.0;
                self.open_menu_board = None;
            }
            HitTarget::SelectProject(id) => {
                self.selection = Selection::Project(id);
                self.scroll_y = 0.0;
                self.open_menu_board = None;
            }
            HitTarget::NewProject => {
                self.open_menu_board = None;
                self.dialog = Some(Dialog::Prompt {
                    title: "Nuevo proyecto".into(),
                    value: String::new(),
                    confirm_label: "Crear".into(),
                    kind: PromptKind::NewProject,
                });
            }
            HitTarget::RenameProject(id) => {
                if let Some(p) = self.workspace.find_project(&id) {
                    self.dialog = Some(Dialog::Prompt {
                        title: "Renombrar proyecto".into(),
                        value: p.name.clone(),
                        confirm_label: "Guardar".into(),
                        kind: PromptKind::RenameProject(id),
                    });
                }
            }
            HitTarget::RecolorProject(id) => {
                if let Some(p) = self.workspace.find_project(&id) {
                    self.dialog = Some(Dialog::ChooseColor {
                        project_id: id,
                        current: p.color.clone(),
                    });
                }
            }
            HitTarget::DeleteProject(id) => {
                if let Some(p) = self.workspace.find_project(&id) {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Eliminar proyecto".into(),
                        message: format!("Se eliminará «{}» y todas sus pizarras ({}).", p.name, p.boards.len()),
                        confirm_label: "Eliminar".into(),
                        kind: ConfirmKind::DeleteProject(id),
                    });
                }
            }
            HitTarget::OpenWorkspaceFolder => return Some(ManagerAction::OpenWorkspaceFolder),
            HitTarget::ToggleTheme => {
                self.theme = self.theme.toggle();
                return Some(ManagerAction::ToggleTheme(self.theme));
            }
            HitTarget::SearchBox => {}
            HitTarget::ClearSearch => {
                self.query.clear();
                self.scroll_y = 0.0;
            }
            HitTarget::NewBoard => {
                self.open_menu_board = None;
                if let Some(project) = self.active_or_first_project() {
                    let next_num = project.boards.len() + 1;
                    self.dialog = Some(Dialog::Prompt {
                        title: "Nueva pizarra".into(),
                        value: format!("Pizarra {next_num}"),
                        confirm_label: "Crear".into(),
                        kind: PromptKind::NewBoard(project.id.clone()),
                    });
                } else {
                    self.dialog = Some(Dialog::Prompt {
                        title: "Nuevo proyecto".into(),
                        value: String::new(),
                        confirm_label: "Crear".into(),
                        kind: PromptKind::NewProject,
                    });
                }
            }
            HitTarget::ImportBoards => {
                self.open_menu_board = None;
                if let Some(project) = self.active_or_first_project() {
                    return Some(ManagerAction::ImportBoards(project.id.clone()));
                }
                self.notice = Some("Crea un proyecto primero para importar pizarras.".into());
            }
            HitTarget::OpenBoard(id) => {
                self.open_menu_board = None;
                return Some(ManagerAction::OpenBoard(id));
            }
            HitTarget::CardMenuBtn(id) => {
                if self.open_menu_board.as_deref() == Some(id.as_str()) {
                    self.open_menu_board = None;
                } else {
                    self.open_menu_board = Some(id);
                    self.menu_anchor = pos;
                }
            }
            HitTarget::MenuAction(choice) => {
                self.open_menu_board = None;
                return self.handle_menu_choice(choice);
            }
            HitTarget::DialogBackdrop | HitTarget::DialogCancel => {
                self.dialog = None;
            }
            HitTarget::DialogInput => {}
            HitTarget::DialogConfirm => return self.confirm_dialog(),
            HitTarget::DialogColor(color) => {
                if let Some(Dialog::ChooseColor { project_id, .. }) = self.dialog.take() {
                    return Some(ManagerAction::RecolorProject { project_id, color });
                }
            }
            HitTarget::DialogPickProject(target_project_id) => {
                if let Some(Dialog::ChooseProject { board_id, .. }) = self.dialog.take() {
                    return Some(ManagerAction::MoveBoard { board_id, target_project_id });
                }
            }
        }
        None
    }

    fn handle_menu_choice(&mut self, choice: MenuChoice) -> Option<ManagerAction> {
        match choice {
            MenuChoice::Open(id) => Some(ManagerAction::OpenBoard(id)),
            MenuChoice::Rename(id) => {
                if let Some(entry) = self.workspace.find_board(&id) {
                    self.dialog = Some(Dialog::Prompt {
                        title: "Renombrar pizarra".into(),
                        value: entry.board.name.clone(),
                        confirm_label: "Guardar".into(),
                        kind: PromptKind::RenameBoard(id),
                    });
                }
                None
            }
            MenuChoice::Duplicate(id) => Some(ManagerAction::DuplicateBoard(id)),
            MenuChoice::Move(id) => {
                if self.workspace.projects.len() < 2 {
                    self.notice = Some("Necesitas al menos dos proyectos para mover pizarras.".into());
                    return None;
                }
                if let Some(entry) = self.workspace.find_board(&id) {
                    self.dialog = Some(Dialog::ChooseProject {
                        board_id: id,
                        current_project_id: entry.project.id.clone(),
                    });
                }
                None
            }
            MenuChoice::Export(id) => Some(ManagerAction::ExportBoard(id)),
            MenuChoice::Delete(id) => {
                if let Some(entry) = self.workspace.find_board(&id) {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Eliminar pizarra".into(),
                        message: format!("Se eliminará «{}» de forma permanente.", entry.board.name),
                        confirm_label: "Eliminar".into(),
                        kind: ConfirmKind::DeleteBoard(id),
                    });
                }
                None
            }
        }
    }

    fn confirm_dialog(&mut self) -> Option<ManagerAction> {
        let dialog = self.dialog.take()?;
        match dialog {
            Dialog::Prompt { value, kind, .. } => {
                let trimmed = value.trim().to_owned();
                if trimmed.is_empty() {
                    return None;
                }
                match kind {
                    PromptKind::NewProject => Some(ManagerAction::CreateProject(trimmed)),
                    PromptKind::RenameProject(project_id) => {
                        Some(ManagerAction::RenameProject { project_id, name: trimmed })
                    }
                    PromptKind::NewBoard(project_id) => {
                        Some(ManagerAction::CreateBoard { project_id, name: trimmed })
                    }
                    PromptKind::RenameBoard(board_id) => {
                        Some(ManagerAction::RenameBoard { board_id, name: trimmed })
                    }
                }
            }
            Dialog::Confirm { kind, .. } => match kind {
                ConfirmKind::DeleteProject(id) => Some(ManagerAction::DeleteProject(id)),
                ConfirmKind::DeleteBoard(id) => Some(ManagerAction::DeleteBoard(id)),
            },
            Dialog::ChooseColor { .. } | Dialog::ChooseProject { .. } => None,
        }
    }

    fn active_or_first_project(&self) -> Option<&Project> {
        match &self.selection {
            Selection::Project(id) => self.workspace.find_project(id),
            Selection::Recent => self.workspace.projects.first(),
        }
    }

    fn visible_entries(&self) -> Vec<(&Project, &Board)> {
        let raw: Vec<(&Project, &Board)> = match &self.selection {
            Selection::Project(id) => match self.workspace.find_project(id) {
                Some(p) => p.boards.iter().map(|b| (p, b)).collect(),
                None => Vec::new(),
            },
            Selection::Recent => self
                .workspace
                .recent_boards(RECENT_LIMIT)
                .into_iter()
                .map(|e| (e.project, e.board))
                .collect(),
        };
        let needle = self.query.trim().to_lowercase();
        if needle.is_empty() {
            raw
        } else {
            raw.into_iter().filter(|(_, b)| b.name.to_lowercase().contains(&needle)).collect()
        }
    }

    pub fn frame(&mut self) -> Frame {
        let mut hits = Vec::new();
        let cursor = self.cursor;
        let has_dialog = self.dialog.is_some();
        let pal = Palette::for_theme(self.theme);
        let mut ops = Vec::new();
        let w = self.size.width.max(320.0);
        let h = self.size.height.max(240.0);

        // Sidebar
        let sidebar_rect = Rect::new(0.0, 0.0, SIDEBAR_W, h);
        fill_rect(&mut ops, sidebar_rect, pal.sidebar);
        line(&mut ops, Point::new(SIDEBAR_W, 0.0), Point::new(SIDEBAR_W, h), pal.border, 1.0);

        // Sidebar header + theme button
        draw_text(&mut ops, "Boards", Point::new(18.0, 34.0), 20.0, pal.text);
        let theme_rect = Rect::new(SIDEBAR_W - 86.0, 14.0, SIDEBAR_W - 14.0, 42.0);
        let theme_label = if self.theme.is_dark() { "Oscuro" } else { "Claro" };
        draw_button(&mut hits, &mut ops, theme_rect, theme_label, pal.surface, pal.border, pal.text, HitTarget::ToggleTheme);

        // Recent item
        let mut y = 64.0;
        let recent_rect = Rect::new(12.0, y, SIDEBAR_W - 12.0, y + 36.0);
        let is_recent = matches!(self.selection, Selection::Recent);
        draw_nav_row(&mut hits, cursor, has_dialog, &mut ops, recent_rect, "Recientes", None, is_recent, &pal, HitTarget::SelectRecent);
        y += 48.0;

        // Projects heading
        draw_text(&mut ops, "PROYECTOS", Point::new(18.0, y + 12.0), 11.0, pal.muted);
        y += 22.0;

        let max_projects_y = h - 104.0;
        for project in &self.workspace.projects {
            if y + 36.0 > max_projects_y {
                break;
            }
            let row = Rect::new(12.0, y, SIDEBAR_W - 12.0, y + 34.0);
            let active = matches!(&self.selection, Selection::Project(id) if id == &project.id);
            let dot = Color::parse(&project.color).unwrap_or(pal.accent);
            let label = format!("{} ({})", truncate(&project.name, 16), project.boards.len());
            draw_nav_row(&mut hits, cursor, has_dialog, 
                &mut ops,
                row,
                &label,
                Some(dot),
                active,
                &pal,
                HitTarget::SelectProject(project.id.clone()),
            );
            y += 38.0;
        }

        let new_proj_rect = Rect::new(12.0, h - 94.0, SIDEBAR_W - 12.0, h - 58.0);
        draw_button(&mut hits, 
            &mut ops,
            new_proj_rect,
            "+ Nuevo proyecto",
            pal.surface,
            pal.border,
            pal.text,
            HitTarget::NewProject,
        );

        let folder_rect = Rect::new(12.0, h - 48.0, SIDEBAR_W - 12.0, h - 14.0);
        draw_button(&mut hits, 
            &mut ops,
            folder_rect,
            "Carpeta de datos",
            pal.surface_alt,
            pal.border,
            pal.muted,
            HitTarget::OpenWorkspaceFolder,
        );

        // Topbar
        let main_x = SIDEBAR_W;
        let main_w = (w - SIDEBAR_W).max(200.0);
        fill_rect(&mut ops, Rect::new(main_x, 0.0, w, TOPBAR_H), pal.bg);
        line(&mut ops, Point::new(main_x, TOPBAR_H), Point::new(w, TOPBAR_H), pal.border, 1.0);

        match &self.selection {
            Selection::Recent => {
                draw_text(&mut ops, "Recientes", Point::new(main_x + PAD, 42.0), 22.0, pal.text);
            }
            Selection::Project(id) => {
                if let Some(p) = self.workspace.find_project(id) {
                    let dot = Color::parse(&p.color).unwrap_or(pal.accent);
                    fill_circle(&mut ops, Point::new(main_x + PAD + 6.0, 35.0), 6.0, dot);
                    let title = truncate(&p.name, 22);
                    draw_text(&mut ops, &title, Point::new(main_x + PAD + 20.0, 42.0), 22.0, pal.text);
                    let tx = main_x + PAD + 28.0 + text::line_width(&title, UI_FONT, 22.0);
                    if tx + 190.0 < w - 380.0 {
                        draw_button(&mut hits, 
                            &mut ops,
                            Rect::new(tx, 20.0, tx + 78.0, 48.0),
                            "Nombre",
                            pal.surface,
                            pal.border,
                            pal.muted,
                            HitTarget::RenameProject(p.id.clone()),
                        );
                        draw_button(&mut hits, 
                            &mut ops,
                            Rect::new(tx + 84.0, 20.0, tx + 140.0, 48.0),
                            "Color",
                            pal.surface,
                            pal.border,
                            pal.muted,
                            HitTarget::RecolorProject(p.id.clone()),
                        );
                        draw_button(&mut hits, 
                            &mut ops,
                            Rect::new(tx + 146.0, 20.0, tx + 216.0, 48.0),
                            "Eliminar",
                            pal.surface,
                            pal.border,
                            pal.danger,
                            HitTarget::DeleteProject(p.id.clone()),
                        );
                    }
                }
            }
        }

        // Topbar right controls: Search + Import + New Board
        let new_board_rect = Rect::new(w - 148.0, 17.0, w - 18.0, 51.0);
        draw_button(&mut hits, 
            &mut ops,
            new_board_rect,
            "+ Nueva pizarra",
            pal.accent,
            pal.accent,
            pal.accent_text,
            HitTarget::NewBoard,
        );

        let import_rect = Rect::new(w - 242.0, 17.0, w - 156.0, 51.0);
        draw_button(&mut hits, 
            &mut ops,
            import_rect,
            "Importar",
            pal.surface,
            pal.border,
            pal.text,
            HitTarget::ImportBoards,
        );

        let search_w = (main_w - 520.0).clamp(150.0, 240.0);
        let search_rect = Rect::new(w - 252.0 - search_w, 17.0, w - 252.0, 51.0);
        let search_border = if self.search_focused { pal.accent } else { pal.border };
        fill_round_rect(&mut ops, search_rect, 8.0, pal.surface);
        stroke_round_rect(&mut ops, search_rect, 8.0, search_border, if self.search_focused { 1.5 } else { 1.0 });
        hits.push(HitRegion { rect: search_rect, target: HitTarget::SearchBox });
        if self.query.is_empty() {
            let ph = if self.search_focused { "Escribe para buscar..." } else { "Buscar pizarras..." };
            draw_text(&mut ops, ph, Point::new(search_rect.x0 + 10.0, 39.0), 13.0, pal.muted);
        } else {
            let q = truncate(&self.query, 20);
            draw_text(&mut ops, &q, Point::new(search_rect.x0 + 10.0, 39.0), 13.0, pal.text);
            let clear_rect = Rect::new(search_rect.x1 - 24.0, search_rect.y0 + 6.0, search_rect.x1 - 4.0, search_rect.y1 - 6.0);
            draw_text(&mut ops, "×", Point::new(clear_rect.x0 + 5.0, 39.0), 15.0, pal.muted);
            hits.push(HitRegion { rect: clear_rect, target: HitTarget::ClearSearch });
        }

        // Grid area
        let entries: Vec<(String, String, String, u64)> = self
            .visible_entries()
            .into_iter()
            .map(|(p, b)| (p.name.clone(), b.id.clone(), b.name.clone(), b.updated_at))
            .collect();
        let grid_w = (main_w - PAD * 2.0).max(CARD_MIN_W);
        let cols = (((grid_w + CARD_GAP) / (CARD_MIN_W + CARD_GAP)).floor() as usize).max(1);
        let card_w = ((grid_w - CARD_GAP * (cols as f64 - 1.0)) / cols as f64).max(CARD_MIN_W);
        let rows = entries.len().div_ceil(cols);
        self.content_height = if entries.is_empty() {
            0.0
        } else {
            PAD * 2.0 + rows as f64 * CARD_H + (rows.saturating_sub(1)) as f64 * CARD_GAP
        };
        self.clamp_scroll();

        if entries.is_empty() {
            let msg = if self.workspace.projects.is_empty() {
                "Crea tu primer proyecto en la barra lateral para empezar."
            } else if !self.query.trim().is_empty() {
                "No hay pizarras que coincidan con la búsqueda."
            } else {
                "No hay pizarras aquí todavía. Usa «+ Nueva pizarra» o «Importar»."
            };
            draw_text(&mut ops, msg, Point::new(main_x + PAD, TOPBAR_H + 46.0), 15.0, pal.muted);
        } else {
            for (idx, (proj_name, board_id, board_name, updated_at)) in entries.iter().enumerate() {
                let col = idx % cols;
                let row = idx / cols;
                let cx = main_x + PAD + col as f64 * (card_w + CARD_GAP);
                let cy = TOPBAR_H + PAD + row as f64 * (CARD_H + CARD_GAP) - self.scroll_y;
                if cy + CARD_H < TOPBAR_H || cy > h {
                    continue;
                }
                let card_rect = Rect::new(cx, cy, cx + card_w, cy + CARD_H);
                let hovered = card_rect.contains(self.cursor) && self.dialog.is_none();
                fill_round_rect(&mut ops, card_rect, 10.0, pal.surface);
                stroke_round_rect(
                    &mut ops,
                    card_rect,
                    10.0,
                    if hovered { pal.accent } else { pal.border },
                    if hovered { 1.5 } else { 1.0 },
                );
                hits.push(HitRegion {
                    rect: card_rect,
                    target: HitTarget::OpenBoard(board_id.clone()),
                });

                // Thumbnail box
                let thumb_rect = Rect::new(cx + 8.0, cy + 8.0, cx + card_w - 8.0, cy + 8.0 + THUMB_H);
                fill_round_rect(&mut ops, thumb_rect, 6.0, pal.surface_alt);
                let thumb_path = thumb_rect.to_path(0.1);
                ops.push(DrawOp {
                    path: thumb_path,
                    color: pal.surface_alt,
                    style: Style::Image {
                        file_id: format!("thumb:{board_id}"),
                        crop: None,
                        flip_x: false,
                        flip_y: false,
                    },
                });

                // Card title & metadata
                let title_y = cy + THUMB_H + 30.0;
                draw_text(&mut ops, &truncate(board_name, 22), Point::new(cx + 12.0, title_y), 14.0, pal.text);

                let meta_y = cy + THUMB_H + 52.0;
                let rel = format_time_ago(self.now_ms, *updated_at);
                let meta = if matches!(self.selection, Selection::Recent) {
                    format!("{} · {}", truncate(proj_name, 14), rel)
                } else {
                    rel
                };
                draw_text(&mut ops, &meta, Point::new(cx + 12.0, meta_y), 11.5, pal.muted);

                // Card menu button
                let menu_btn = Rect::new(cx + card_w - 36.0, cy + THUMB_H + 14.0, cx + card_w - 10.0, cy + THUMB_H + 40.0);
                let btn_bg = if menu_btn.contains(self.cursor) { pal.hover } else { Color::TRANSPARENT };
                fill_round_rect(&mut ops, menu_btn, 6.0, btn_bg);
                draw_text(&mut ops, "...", Point::new(menu_btn.x0 + 7.0, menu_btn.y0 + 17.0), 14.0, pal.muted);
                hits.push(HitRegion {
                    rect: menu_btn,
                    target: HitTarget::CardMenuBtn(board_id.clone()),
                });
            }
        }

        // Open card context menu
        if let Some(board_id) = self.open_menu_board.clone() {
            let mw = 160.0;
            let item_h = 32.0;
            let items = [
                ("Abrir", MenuChoice::Open(board_id.clone()), false),
                ("Renombrar", MenuChoice::Rename(board_id.clone()), false),
                ("Duplicar", MenuChoice::Duplicate(board_id.clone()), false),
                ("Mover a...", MenuChoice::Move(board_id.clone()), false),
                ("Exportar", MenuChoice::Export(board_id.clone()), false),
                ("Eliminar", MenuChoice::Delete(board_id), true),
            ];
            let mh = items.len() as f64 * item_h + 8.0;
            let mx = self.menu_anchor.x.min(w - mw - 12.0).max(12.0);
            let my = self.menu_anchor.y.min(h - mh - 12.0).max(12.0);
            let menu_rect = Rect::new(mx, my, mx + mw, my + mh);
            fill_round_rect(&mut ops, menu_rect, 8.0, pal.surface);
            stroke_round_rect(&mut ops, menu_rect, 8.0, pal.border, 1.0);
            for (i, (label, choice, danger)) in items.into_iter().enumerate() {
                let iy = my + 4.0 + i as f64 * item_h;
                let row = Rect::new(mx + 4.0, iy, mx + mw - 4.0, iy + item_h);
                if row.contains(self.cursor) {
                    fill_round_rect(&mut ops, row, 5.0, pal.hover);
                }
                let color = if danger { pal.danger } else { pal.text };
                draw_text(&mut ops, label, Point::new(row.x0 + 10.0, iy + 21.0), 13.0, color);
                hits.push(HitRegion { rect: row, target: HitTarget::MenuAction(choice) });
            }
        }

        // Notice toast
        if let Some(msg) = &self.notice {
            let tw = (text::line_width(msg, UI_FONT, 13.0) + 28.0).min(w - 40.0);
            let toast = Rect::new(w - tw - 20.0, h - 54.0, w - 20.0, h - 20.0);
            fill_round_rect(&mut ops, toast, 8.0, pal.surface);
            stroke_round_rect(&mut ops, toast, 8.0, pal.accent, 1.2);
            draw_text(&mut ops, msg, Point::new(toast.x0 + 14.0, toast.y0 + 22.0), 13.0, pal.text);
        }

        // Modal dialog
        if let Some(dialog) = self.dialog.clone() {
            let backdrop = Rect::new(0.0, 0.0, w, h);
            fill_rect(&mut ops, backdrop, Color::rgba(0, 0, 0, 110));
            hits.push(HitRegion { rect: backdrop, target: HitTarget::DialogBackdrop });

            let dw = 380.0_f64.min(w - 32.0);
            let dh = match &dialog {
                Dialog::Prompt { .. } => 168.0,
                Dialog::ChooseColor { .. } => 148.0,
                Dialog::Confirm { .. } => 164.0,
                Dialog::ChooseProject { .. } => {
                    let count = self.workspace.projects.len().saturating_sub(1).max(1) as f64;
                    (96.0 + count * 38.0).min(h - 60.0)
                }
            };
            let dx = (w - dw) / 2.0;
            let dy = (h - dh) / 2.0;
            let box_rect = Rect::new(dx, dy, dx + dw, dy + dh);
            fill_round_rect(&mut ops, box_rect, 12.0, pal.surface);
            stroke_round_rect(&mut ops, box_rect, 12.0, pal.border, 1.0);
            hits.push(HitRegion { rect: box_rect, target: HitTarget::DialogInput });

            match dialog {
                Dialog::Prompt { title, value, confirm_label, .. } => {
                    draw_text(&mut ops, &title, Point::new(dx + 20.0, dy + 34.0), 17.0, pal.text);
                    let input_rect = Rect::new(dx + 20.0, dy + 54.0, dx + dw - 20.0, dy + 92.0);
                    fill_round_rect(&mut ops, input_rect, 8.0, pal.bg);
                    stroke_round_rect(&mut ops, input_rect, 8.0, pal.accent, 1.5);
                    let display = format!("{value}|");
                    draw_text(&mut ops, &truncate(&display, 38), Point::new(input_rect.x0 + 10.0, dy + 78.0), 14.0, pal.text);
                    let cancel_rect = Rect::new(dx + dw - 196.0, dy + dh - 50.0, dx + dw - 110.0, dy + dh - 16.0);
                    let ok_rect = Rect::new(dx + dw - 102.0, dy + dh - 50.0, dx + dw - 20.0, dy + dh - 16.0);
                    draw_button(&mut hits, &mut ops, cancel_rect, "Cancelar", pal.surface_alt, pal.border, pal.text, HitTarget::DialogCancel);
                    draw_button(&mut hits, &mut ops, ok_rect, &confirm_label, pal.accent, pal.accent, pal.accent_text, HitTarget::DialogConfirm);
                }
                Dialog::ChooseColor { current, .. } => {
                    draw_text(&mut ops, "Color del proyecto", Point::new(dx + 20.0, dy + 34.0), 17.0, pal.text);
                    for (i, hex) in PROJECT_COLORS.iter().enumerate() {
                        let sx = dx + 24.0 + i as f64 * 46.0;
                        let sy = dy + 58.0;
                        let sw = Rect::new(sx, sy, sx + 34.0, sy + 34.0);
                        let color = Color::parse(hex).unwrap_or(pal.accent);
                        fill_round_rect(&mut ops, sw, 17.0, color);
                        if current.eq_ignore_ascii_case(hex) {
                            stroke_round_rect(&mut ops, sw, 17.0, pal.text, 2.5);
                        }
                        hits.push(HitRegion {
                            rect: sw,
                            target: HitTarget::DialogColor((*hex).to_owned()),
                        });
                    }
                    let cancel_rect = Rect::new(dx + dw - 106.0, dy + dh - 48.0, dx + dw - 20.0, dy + dh - 14.0);
                    draw_button(&mut hits, &mut ops, cancel_rect, "Cerrar", pal.surface_alt, pal.border, pal.text, HitTarget::DialogCancel);
                }
                Dialog::ChooseProject { current_project_id, .. } => {
                    draw_text(&mut ops, "Mover a proyecto", Point::new(dx + 20.0, dy + 34.0), 17.0, pal.text);
                    let mut py = dy + 52.0;
                    for p in &self.workspace.projects {
                        if p.id == current_project_id || py + 34.0 > dy + dh - 52.0 {
                            continue;
                        }
                        let row = Rect::new(dx + 20.0, py, dx + dw - 20.0, py + 32.0);
                        let dot = Color::parse(&p.color).unwrap_or(pal.accent);
                        draw_nav_row(&mut hits, cursor, has_dialog, 
                            &mut ops,
                            row,
                            &truncate(&p.name, 30),
                            Some(dot),
                            false,
                            &pal,
                            HitTarget::DialogPickProject(p.id.clone()),
                        );
                        py += 38.0;
                    }
                    let cancel_rect = Rect::new(dx + dw - 106.0, dy + dh - 48.0, dx + dw - 20.0, dy + dh - 14.0);
                    draw_button(&mut hits, &mut ops, cancel_rect, "Cancelar", pal.surface_alt, pal.border, pal.text, HitTarget::DialogCancel);
                }
                Dialog::Confirm { title, message, confirm_label, .. } => {
                    draw_text(&mut ops, &title, Point::new(dx + 20.0, dy + 34.0), 17.0, pal.text);
                    draw_text(&mut ops, &truncate(&message, 46), Point::new(dx + 20.0, dy + 72.0), 13.5, pal.muted);
                    let cancel_rect = Rect::new(dx + dw - 196.0, dy + dh - 50.0, dx + dw - 110.0, dy + dh - 16.0);
                    let ok_rect = Rect::new(dx + dw - 102.0, dy + dh - 50.0, dx + dw - 20.0, dy + dh - 16.0);
                    draw_button(&mut hits, &mut ops, cancel_rect, "Cancelar", pal.surface_alt, pal.border, pal.text, HitTarget::DialogCancel);
                    draw_button(&mut hits, &mut ops, ok_rect, &confirm_label, pal.danger, pal.danger, pal.accent_text, HitTarget::DialogConfirm);
                }
            }
        }

        self.hits = hits;
        Frame {
            background: pal.bg,
            items: vec![Item { transform: Affine::IDENTITY, opacity: 1.0, ops: Rc::new(ops) }],
        }
    }


}

#[allow(clippy::too_many_arguments)]
fn draw_nav_row(
    hits: &mut Vec<HitRegion>,
    cursor: Point,
    has_dialog: bool,
    ops: &mut Vec<DrawOp>,
    rect: Rect,
    label: &str,
    dot: Option<Color>,
    active: bool,
    pal: &Palette,
    target: HitTarget,
) {
    let hovered = rect.contains(cursor) && !has_dialog;
        let bg = if active {
            pal.selected
        } else if hovered {
            pal.hover
        } else {
            Color::TRANSPARENT
        };
        if !bg.is_transparent() {
            fill_round_rect(ops, rect, 7.0, bg);
        }
        let text_x = if let Some(c) = dot {
            fill_circle(ops, Point::new(rect.x0 + 14.0, (rect.y0 + rect.y1) / 2.0), 5.0, c);
            rect.x0 + 26.0
        } else {
            rect.x0 + 12.0
        };
        let color = if active { pal.accent } else { pal.text };
        draw_text(ops, label, Point::new(text_x, (rect.y0 + rect.y1) / 2.0 + 4.5), 13.5, color);
        hits.push(HitRegion { rect, target });
    }

#[allow(clippy::too_many_arguments)]
fn draw_button(
    hits: &mut Vec<HitRegion>,
    ops: &mut Vec<DrawOp>,
    rect: Rect,
    label: &str,
    bg: Color,
    border: Color,
    fg: Color,
    target: HitTarget,
) {
        fill_round_rect(ops, rect, 8.0, bg);
        stroke_round_rect(ops, rect, 8.0, border, 1.0);
        let tw = text::line_width(label, UI_FONT, 13.0);
        let tx = rect.x0 + ((rect.width() - tw) / 2.0).max(6.0);
        let ty = (rect.y0 + rect.y1) / 2.0 + 4.5;
        draw_text(ops, label, Point::new(tx, ty), 13.0, fg);
        hits.push(HitRegion { rect, target });
}

fn fill_rect(ops: &mut Vec<DrawOp>, rect: Rect, color: Color) {
    ops.push(DrawOp::fill(rect.to_path(0.1), color));
}

fn fill_round_rect(ops: &mut Vec<DrawOp>, rect: Rect, radius: f64, color: Color) {
    let rr = RoundedRect::from_rect(rect, radius);
    ops.push(DrawOp::fill(rr.to_path(0.1), color));
}

fn stroke_round_rect(ops: &mut Vec<DrawOp>, rect: Rect, radius: f64, color: Color, width: f64) {
    let rr = RoundedRect::from_rect(rect, radius);
    ops.push(DrawOp::stroke(rr.to_path(0.1), color, width));
}

fn fill_circle(ops: &mut Vec<DrawOp>, center: Point, radius: f64, color: Color) {
    let circle = kurbo::Circle::new(center, radius);
    ops.push(DrawOp::fill(circle.to_path(0.1), color));
}

fn line(ops: &mut Vec<DrawOp>, a: Point, b: Point, color: Color, width: f64) {
    let mut path = BezPath::new();
    path.move_to(a);
    path.line_to(b);
    ops.push(DrawOp::stroke(path, color, width));
}

fn draw_text(ops: &mut Vec<DrawOp>, s: &str, baseline: Point, size: f64, color: Color) {
    let mut path = BezPath::new();
    text::append_line(s, UI_FONT, size, baseline, &mut path);
    if !path.is_empty() {
        ops.push(DrawOp::fill(path, color));
    }
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() { format!("{head}…") } else { head }
}

fn format_time_ago(now_ms: u64, then_ms: u64) -> String {
    let diff_s = now_ms.saturating_sub(then_ms) / 1000;
    if diff_s < 60 {
        "ahora".into()
    } else if diff_s < 3600 {
        format!("hace {} min", diff_s / 60)
    } else if diff_s < 86_400 {
        format!("hace {} h", diff_s / 3600)
    } else {
        format!("hace {} d", diff_s / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_cancels_new_project_dialog() {
        let mut mgr = ManagerState::new(Workspace::default(), Size::new(1000.0, 700.0), ThemeMode::Light, 1_000_000);
        let _ = mgr.frame();
        let new_proj = mgr.hits.iter().find(|h| matches!(h.target, HitTarget::NewProject)).unwrap().rect;
        assert!(mgr.click(new_proj.center()).is_none());
        assert!(mgr.has_modal_or_focus());

        mgr.text_input("Matemáticas");
        let action = mgr.key_down(Key::Enter);
        assert_eq!(action, Some(ManagerAction::CreateProject("Matemáticas".into())));
        assert!(!mgr.has_modal_or_focus());
    }
}
