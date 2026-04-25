#![allow(dead_code)]

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryAgentSnapshot {
    pub path: PathBuf,
    pub contents: String,
}

pub trait MemoryAgent {
    fn backend_label(&self) -> &str;
    fn read_long_term_memory(&self) -> io::Result<MemoryAgentSnapshot>;
    fn append_long_term_note(&self, note: &str) -> io::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMemoryAgent {
    path: PathBuf,
}

impl FileMemoryAgent {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl MemoryAgent for FileMemoryAgent {
    fn backend_label(&self) -> &str {
        "file"
    }

    fn read_long_term_memory(&self) -> io::Result<MemoryAgentSnapshot> {
        let contents = if self.path.is_file() {
            fs::read_to_string(&self.path)?
        } else {
            String::new()
        };

        Ok(MemoryAgentSnapshot {
            path: self.path.clone(),
            contents,
        })
    }

    fn append_long_term_note(&self, note: &str) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut contents = if self.path.is_file() {
            fs::read_to_string(&self.path)?
        } else {
            "# Long-Term Memory\n\nDurable notes promoted by the runtime.\n".to_string()
        };
        if !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(note);
        fs::write(&self.path, contents)
    }
}

#[derive(Debug, Clone)]
pub struct StubMemoryAgent {
    backend: String,
    snapshot: RefCell<MemoryAgentSnapshot>,
    appended_notes: RefCell<Vec<String>>,
}

impl StubMemoryAgent {
    pub fn new(backend: impl Into<String>, snapshot: MemoryAgentSnapshot) -> Self {
        Self {
            backend: backend.into(),
            snapshot: RefCell::new(snapshot),
            appended_notes: RefCell::new(Vec::new()),
        }
    }

    pub fn appended_notes(&self) -> Vec<String> {
        self.appended_notes.borrow().clone()
    }
}

impl MemoryAgent for StubMemoryAgent {
    fn backend_label(&self) -> &str {
        &self.backend
    }

    fn read_long_term_memory(&self) -> io::Result<MemoryAgentSnapshot> {
        Ok(self.snapshot.borrow().clone())
    }

    fn append_long_term_note(&self, note: &str) -> io::Result<()> {
        self.appended_notes.borrow_mut().push(note.to_string());
        let mut snapshot = self.snapshot.borrow_mut();
        if !snapshot.contents.ends_with('\n') && !snapshot.contents.is_empty() {
            snapshot.contents.push('\n');
        }
        snapshot.contents.push_str(note);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ))
    }

    #[test]
    fn file_memory_agent_reads_and_appends_long_term_notes() {
        let path = temp_path("memory-agent-file");
        let agent = FileMemoryAgent::new(&path);

        let initial = agent.read_long_term_memory().expect("read should succeed");
        assert_eq!(initial.path, path);
        assert!(initial.contents.is_empty());

        agent
            .append_long_term_note("- [2026-04-21T00:00:00Z] remember this\n")
            .expect("append should succeed");

        let updated = agent.read_long_term_memory().expect("read should succeed");
        assert!(updated.contents.contains("remember this"));
    }

    #[test]
    fn stub_memory_agent_exposes_a_future_memory_worker_shape() {
        let agent = StubMemoryAgent::new(
            "stub",
            MemoryAgentSnapshot {
                path: PathBuf::from("stub://memory"),
                contents: "seed".to_string(),
            },
        );

        agent
            .append_long_term_note("note from worker")
            .expect("stub append should succeed");

        let snapshot = agent
            .read_long_term_memory()
            .expect("stub read should succeed");
        assert_eq!(agent.backend_label(), "stub");
        assert!(snapshot.contents.contains("seed"));
        assert!(snapshot.contents.contains("note from worker"));
        assert_eq!(agent.appended_notes(), vec!["note from worker".to_string()]);
    }
}
