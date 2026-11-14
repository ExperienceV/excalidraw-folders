//! Platform-independent workspace index model (`workspace.json`).
//!
//! Uses the exact JSON schema (`version: 1`, camelCase timestamps) of the
//! desktop `Boards` app so workspaces are 100% interchangeable.

use serde::{Deserialize, Serialize};

pub const PROJECT_COLORS: [&str; 7] = [
    "#c4ed62", "#7dd3fc", "#fca5a5", "#fcd34d", "#c4b5fd", "#86efac", "#f9a8d4",
];

pub const MAX_NAME_LENGTH: usize = 120;
pub const RECENT_LIMIT: usize = 24;

pub fn clean_name(name: &str, fallback: &str) -> String {
    let trimmed: String = name.trim().chars().take(MAX_NAME_LENGTH).collect();
    if trimmed.is_empty() { fallback.to_owned() } else { trimmed }
}

pub fn is_valid_hex_color(color: &str) -> bool {
    let bytes = color.as_bytes();
    bytes.len() == 7 && bytes[0] == b'#' && bytes[1..].iter().all(|b| b.is_ascii_hexdigit())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub color: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub boards: Vec<Board>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<Project>,
}

fn default_version() -> u32 {
    1
}

impl Default for Workspace {
    fn default() -> Self {
        Self { version: 1, projects: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct BoardEntry<'a> {
    pub project: &'a Project,
    pub board: &'a Board,
}

impl Workspace {
    pub fn find_project(&self, project_id: &str) -> Option<&Project> {
        self.projects.iter().find(|p| p.id == project_id)
    }

    pub fn find_project_mut(&mut self, project_id: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.id == project_id)
    }

    pub fn find_board(&self, board_id: &str) -> Option<BoardEntry<'_>> {
        for project in &self.projects {
            if let Some(board) = project.boards.iter().find(|b| b.id == board_id) {
                return Some(BoardEntry { project, board });
            }
        }
        None
    }

    /// Returns up to `limit` boards sorted by `updated_at` descending.
    pub fn recent_boards(&self, limit: usize) -> Vec<BoardEntry<'_>> {
        let mut all: Vec<BoardEntry<'_>> = self
            .projects
            .iter()
            .flat_map(|project| project.boards.iter().map(move |board| BoardEntry { project, board }))
            .collect();
        all.sort_by(|a, b| b.board.updated_at.cmp(&a.board.updated_at));
        all.truncate(limit);
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_boards_workspace_json() {
        let raw = r##"{
          "version": 1,
          "projects": [
            {
              "id": "p1",
              "name": "Diseño",
              "color": "#c4ed62",
              "createdAt": 1700000000000,
              "updatedAt": 1700000001000,
              "boards": [
                {
                  "id": "b1",
                  "name": "Arquitectura",
                  "createdAt": 1700000000500,
                  "updatedAt": 1700000002000
                }
              ]
            }
          ]
        }"##;
        let ws: Workspace = serde_json::from_str(raw).unwrap();
        assert_eq!(ws.version, 1);
        assert_eq!(ws.projects.len(), 1);
        assert_eq!(ws.projects[0].boards[0].name, "Arquitectura");
        let back = serde_json::to_string(&ws).unwrap();
        assert!(back.contains("\"createdAt\":1700000000000"));
        assert!(back.contains("\"updatedAt\":1700000002000"));
    }

    #[test]
    fn recent_boards_sorts_descending_and_truncates() {
        let ws = Workspace {
            version: 1,
            projects: vec![
                Project {
                    id: "p1".into(),
                    name: "A".into(),
                    color: "#c4ed62".into(),
                    created_at: 1,
                    updated_at: 1,
                    boards: vec![
                        Board { id: "b1".into(), name: "Old".into(), created_at: 1, updated_at: 10 },
                        Board { id: "b2".into(), name: "New".into(), created_at: 1, updated_at: 30 },
                    ],
                },
                Project {
                    id: "p2".into(),
                    name: "B".into(),
                    color: "#7dd3fc".into(),
                    created_at: 1,
                    updated_at: 1,
                    boards: vec![Board { id: "b3".into(), name: "Mid".into(), created_at: 1, updated_at: 20 }],
                },
            ],
        };
        let recent = ws.recent_boards(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].board.id, "b2");
        assert_eq!(recent[1].board.id, "b3");
    }
}
