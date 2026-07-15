use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::{ProjectRow, Settings, VoiceInfo};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  root_path TEXT NOT NULL UNIQUE,
  voice_id TEXT NOT NULL,
  label TEXT NOT NULL,
  stability REAL NOT NULL,
  similarity_boost REAL NOT NULL,
  style REAL NOT NULL,
  speed REAL NOT NULL,
  use_speaker_boost INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS utterances (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL,
  project_id TEXT,
  voice_id TEXT NOT NULL,
  chars INTEGER NOT NULL,
  cached INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS voices (
  voice_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  category TEXT NOT NULL,
  cached_at TEXT NOT NULL
);
"#;

pub struct Db {
    conn: Mutex<Connection>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        conn.execute_batch(SCHEMA).context("init schema")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("db mutex poisoned")
    }

    fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRow> {
        Ok(ProjectRow {
            id: row.get(0)?,
            name: row.get(1)?,
            root_path: row.get(2)?,
            voice_id: row.get(3)?,
            label: row.get(4)?,
            settings: Settings {
                stability: row.get(5)?,
                similarity_boost: row.get(6)?,
                style: row.get(7)?,
                speed: row.get(8)?,
                use_speaker_boost: row.get::<_, i64>(9)? != 0,
            },
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }

    const COLS: &'static str =
        "id,name,root_path,voice_id,label,stability,similarity_boost,style,speed,use_speaker_boost,created_at,updated_at";

    pub fn get_project_by_root(&self, root: &str) -> Result<Option<ProjectRow>> {
        let c = self.conn();
        let sql = format!("SELECT {} FROM projects WHERE root_path = ?1", Self::COLS);
        let mut stmt = c.prepare(&sql)?;
        let mut rows = stmt.query_map(params![root], Self::row_to_project)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn get_project_by_id(&self, id: &str) -> Result<Option<ProjectRow>> {
        let c = self.conn();
        let sql = format!("SELECT {} FROM projects WHERE id = ?1", Self::COLS);
        let mut stmt = c.prepare(&sql)?;
        let mut rows = stmt.query_map(params![id], Self::row_to_project)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn insert_project(&self, p: &ProjectRow) -> Result<()> {
        let c = self.conn();
        c.execute(
            "INSERT INTO projects (id,name,root_path,voice_id,label,stability,similarity_boost,style,speed,use_speaker_boost,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                p.id, p.name, p.root_path, p.voice_id, p.label,
                p.settings.stability, p.settings.similarity_boost, p.settings.style,
                p.settings.speed, p.settings.use_speaker_boost as i64,
                p.created_at, p.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn assign(
        &self,
        id: &str,
        voice_id: &str,
        label: Option<&str>,
        settings: Option<Settings>,
    ) -> Result<bool> {
        let c = self.conn();
        let cur_label;
        let label_ref: &str = match label {
            Some(l) => l,
            None => {
                cur_label = c.query_row(
                    "SELECT label FROM projects WHERE id = ?1",
                    params![id],
                    |r| r.get::<_, String>(0),
                )?;
                &cur_label
            }
        };
        let ts = now();
        let n = if let Some(s) = settings {
            c.execute(
                "UPDATE projects SET voice_id=?1, label=?2, stability=?3, similarity_boost=?4, style=?5, speed=?6, use_speaker_boost=?7, updated_at=?8 WHERE id=?9",
                params![voice_id, label_ref, s.stability, s.similarity_boost, s.style, s.speed, s.use_speaker_boost as i64, ts, id],
            )?
        } else {
            c.execute(
                "UPDATE projects SET voice_id=?1, label=?2, updated_at=?3 WHERE id=?4",
                params![voice_id, label_ref, ts, id],
            )?
        };
        Ok(n > 0)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRow>> {
        let c = self.conn();
        let sql = format!("SELECT {} FROM projects ORDER BY name", Self::COLS);
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_project)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn delete_project(&self, id: &str) -> Result<bool> {
        let c = self.conn();
        let n = c.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    pub fn log_utterance(
        &self,
        project_id: Option<&str>,
        voice_id: &str,
        chars: usize,
        cached: bool,
    ) -> Result<()> {
        let c = self.conn();
        c.execute(
            "INSERT INTO utterances (ts, project_id, voice_id, chars, cached) VALUES (?1,?2,?3,?4,?5)",
            params![now(), project_id, voice_id, chars as i64, cached as i64],
        )?;
        Ok(())
    }

    pub fn utterance_count(&self) -> Result<i64> {
        let c = self.conn();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM utterances", [], |r| r.get(0))?;
        Ok(n)
    }

    pub fn upsert_voices(&self, voices: &[VoiceInfo]) -> Result<()> {
        let c = self.conn();
        let ts = now();
        for v in voices {
            c.execute(
                "INSERT INTO voices (voice_id,name,category,cached_at) VALUES (?1,?2,?3,?4)
                 ON CONFLICT(voice_id) DO UPDATE SET name=excluded.name, category=excluded.category, cached_at=excluded.cached_at",
                params![v.voice_id, v.name, v.category, ts],
            )?;
        }
        Ok(())
    }

    pub fn cached_voice_ids(&self) -> Result<Vec<String>> {
        let c = self.conn();
        let mut stmt = c.prepare("SELECT voice_id FROM voices ORDER BY name")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}
