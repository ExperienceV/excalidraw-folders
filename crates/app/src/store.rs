//! Disk persistence for `Boards` workspaces (`workspace.json`, `boards/*.excalidraw`,
//! `thumbnails/*.png`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use board_core::workspace::{clean_name, is_valid_hex_color, Board, Project, Workspace, PROJECT_COLORS};

static ID_SEQ: AtomicU64 = AtomicU64::new(1);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:016x}-{seq:04x}")
}

fn write_file_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp.{}.{seq}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("rename {}: {e}", path.display())
    })
}

pub struct WorkspaceStore {
    root: PathBuf,
    workspace: Workspace,
}

impl WorkspaceStore {
    pub fn default_root() -> PathBuf {
        if let Ok(custom) = std::env::var("BOARDS_WORKSPACE") {
            if !custom.trim().is_empty() {
                return PathBuf::from(custom);
            }
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.trim().is_empty() {
                return PathBuf::from(xdg).join("Boards").join("workspace");
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join("Boards").join("workspace");
        }
        PathBuf::from(".boards-workspace")
    }

    pub fn open(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(root.join("boards")).map_err(|e| e.to_string())?;
        fs::create_dir_all(root.join("thumbnails")).map_err(|e| e.to_string())?;

        let index_path = root.join("workspace.json");
        let workspace = match fs::read_to_string(&index_path) {
            Ok(raw) => match serde_json::from_str::<Workspace>(&raw) {
                Ok(ws) => ws,
                Err(_) => {
                    let backup = root.join(format!("workspace.json.corrupt-{}", now_ms()));
                    let _ = fs::copy(&index_path, backup);
                    Workspace::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Workspace::default(),
            Err(err) => return Err(format!("read {}: {err}", index_path.display())),
        };

        Ok(Self { root, workspace })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn board_path(&self, board_id: &str) -> PathBuf {
        self.root.join("boards").join(format!("{board_id}.excalidraw"))
    }

    pub fn thumbnail_path(&self, board_id: &str) -> PathBuf {
        self.root.join("thumbnails").join(format!("{board_id}.png"))
    }

    fn persist(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.workspace).map_err(|e| e.to_string())?;
        write_file_atomic(&self.root.join("workspace.json"), json.as_bytes())
    }

    pub fn create_project(&mut self, name: &str) -> Result<Project, String> {
        let now = now_ms();
        let color = PROJECT_COLORS[self.workspace.projects.len() % PROJECT_COLORS.len()].to_owned();
        let project = Project {
            id: new_id(),
            name: clean_name(name, "Proyecto sin título"),
            color,
            created_at: now,
            updated_at: now,
            boards: Vec::new(),
        };
        self.workspace.projects.push(project.clone());
        self.persist()?;
        Ok(project)
    }

    pub fn rename_project(&mut self, project_id: &str, name: &str) -> Result<(), String> {
        let project = self
            .workspace
            .find_project_mut(project_id)
            .ok_or_else(|| format!("Proyecto no encontrado: {project_id}"))?;
        project.name = clean_name(name, &project.name.clone());
        project.updated_at = now_ms();
        self.persist()
    }

    pub fn recolor_project(&mut self, project_id: &str, color: &str) -> Result<(), String> {
        if !is_valid_hex_color(color) {
            return Err(format!("Color inválido: {color}"));
        }
        let project = self
            .workspace
            .find_project_mut(project_id)
            .ok_or_else(|| format!("Proyecto no encontrado: {project_id}"))?;
        project.color = color.to_owned();
        project.updated_at = now_ms();
        self.persist()
    }

    pub fn delete_project(&mut self, project_id: &str) -> Result<(), String> {
        let idx = self
            .workspace
            .projects
            .iter()
            .position(|p| p.id == project_id)
            .ok_or_else(|| format!("Proyecto no encontrado: {project_id}"))?;
        let removed = self.workspace.projects.remove(idx);
        self.persist()?;
        for board in removed.boards {
            let _ = fs::remove_file(self.board_path(&board.id));
            let _ = fs::remove_file(self.thumbnail_path(&board.id));
        }
        Ok(())
    }

    pub fn create_board(&mut self, project_id: &str, name: &str) -> Result<Board, String> {
        let now = now_ms();
        let board = Board {
            id: new_id(),
            name: clean_name(name, "Pizarra sin título"),
            created_at: now,
            updated_at: now,
        };
        let project = self
            .workspace
            .find_project_mut(project_id)
            .ok_or_else(|| format!("Proyecto no encontrado: {project_id}"))?;
        project.boards.push(board.clone());
        project.updated_at = now;
        self.persist()?;
        Ok(board)
    }

    pub fn rename_board(&mut self, board_id: &str, name: &str) -> Result<(), String> {
        let now = now_ms();
        for project in &mut self.workspace.projects {
            if let Some(board) = project.boards.iter_mut().find(|b| b.id == board_id) {
                board.name = clean_name(name, &board.name.clone());
                board.updated_at = now;
                project.updated_at = now;
                return self.persist();
            }
        }
        Err(format!("Pizarra no encontrada: {board_id}"))
    }

    pub fn duplicate_board(&mut self, board_id: &str) -> Result<Board, String> {
        let entry = self
            .workspace
            .find_board(board_id)
            .ok_or_else(|| format!("Pizarra no encontrada: {board_id}"))?;
        let project_id = entry.project.id.clone();
        let copy_name = clean_name(&format!("{} (copia)", entry.board.name), "Pizarra (copia)");
        let copy = self.create_board(&project_id, &copy_name)?;
        let src_scene = self.board_path(board_id);
        if src_scene.exists() {
            let _ = fs::copy(src_scene, self.board_path(&copy.id));
        }
        let src_thumb = self.thumbnail_path(board_id);
        if src_thumb.exists() {
            let _ = fs::copy(src_thumb, self.thumbnail_path(&copy.id));
        }
        Ok(copy)
    }

    pub fn move_board(&mut self, board_id: &str, target_project_id: &str) -> Result<(), String> {
        if self.workspace.find_project(target_project_id).is_none() {
            return Err(format!("Proyecto destino no encontrado: {target_project_id}"));
        }
        let mut extracted: Option<Board> = None;
        let now = now_ms();
        for project in &mut self.workspace.projects {
            if let Some(pos) = project.boards.iter().position(|b| b.id == board_id) {
                if project.id == target_project_id {
                    return Ok(());
                }
                let mut b = project.boards.remove(pos);
                b.updated_at = now;
                project.updated_at = now;
                extracted = Some(b);
                break;
            }
        }
        let board = extracted.ok_or_else(|| format!("Pizarra no encontrada: {board_id}"))?;
        if let Some(target) = self.workspace.find_project_mut(target_project_id) {
            target.boards.push(board);
            target.updated_at = now;
        }
        self.persist()
    }

    pub fn delete_board(&mut self, board_id: &str) -> Result<(), String> {
        let now = now_ms();
        let mut found = false;
        for project in &mut self.workspace.projects {
            if let Some(pos) = project.boards.iter().position(|b| b.id == board_id) {
                project.boards.remove(pos);
                project.updated_at = now;
                found = true;
                break;
            }
        }
        if !found {
            return Err(format!("Pizarra no encontrada: {board_id}"));
        }
        self.persist()?;
        let _ = fs::remove_file(self.board_path(board_id));
        let _ = fs::remove_file(self.thumbnail_path(board_id));
        Ok(())
    }

    pub fn load_board(&self, board_id: &str) -> Result<Option<String>, String> {
        let path = self.board_path(board_id);
        match fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(format!("read {}: {err}", path.display())),
        }
    }

    pub fn save_board(&mut self, board_id: &str, scene_json: &str) -> Result<(), String> {
        write_file_atomic(&self.board_path(board_id), scene_json.as_bytes())?;
        let now = now_ms();
        for project in &mut self.workspace.projects {
            if let Some(board) = project.boards.iter_mut().find(|b| b.id == board_id) {
                board.updated_at = now;
                project.updated_at = now;
                return self.persist();
            }
        }
        Ok(())
    }

    pub fn save_thumbnail(&self, board_id: &str, png_bytes: Option<&[u8]>) -> Result<(), String> {
        let path = self.thumbnail_path(board_id);
        match png_bytes {
            Some(bytes) if !bytes.is_empty() => write_file_atomic(&path, bytes),
            _ => {
                let _ = fs::remove_file(path);
                Ok(())
            }
        }
    }

    pub fn import_board_file(&mut self, project_id: &str, source_path: &Path) -> Result<Board, String> {
        let raw = fs::read_to_string(source_path).map_err(|e| e.to_string())?;
        let stem = source_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Importada");
        let board = self.create_board(project_id, stem)?;
        self.save_board(&board.id, &raw)?;
        Ok(board)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("boards-test-{name}-{}", new_id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn crud_projects_and_boards_persists_and_cleans_files() {
        let dir = temp_workspace("crud");
        let mut store = WorkspaceStore::open(dir.clone()).unwrap();
        let p1 = store.create_project("Proyecto A").unwrap();
        let p2 = store.create_project("Proyecto B").unwrap();
        let b1 = store.create_board(&p1.id, "Idea 1").unwrap();
        store.save_board(&b1.id, r#"{"type":"excalidraw","version":2,"elements":[]}"#).unwrap();
        store.save_thumbnail(&b1.id, Some(b"fake-png")).unwrap();
        assert!(store.board_path(&b1.id).exists());
        assert!(store.thumbnail_path(&b1.id).exists());

        let dup = store.duplicate_board(&b1.id).unwrap();
        assert!(store.board_path(&dup.id).exists());
        store.move_board(&dup.id, &p2.id).unwrap();

        let reloaded = WorkspaceStore::open(dir.clone()).unwrap();
        assert_eq!(reloaded.workspace().find_project(&p1.id).unwrap().boards.len(), 1);
        assert_eq!(reloaded.workspace().find_project(&p2.id).unwrap().boards.len(), 1);

        store.delete_board(&b1.id).unwrap();
        assert!(!store.board_path(&b1.id).exists());
        assert!(!store.thumbnail_path(&b1.id).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_index_is_backed_up_before_reset() {
        let dir = temp_workspace("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("workspace.json"), "{not-valid-json").unwrap();
        let store = WorkspaceStore::open(dir.clone()).unwrap();
        assert!(store.workspace().projects.is_empty());
        let has_backup = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("workspace.json.corrupt-"));
        assert!(has_backup);
        let _ = fs::remove_dir_all(&dir);
    }
}
