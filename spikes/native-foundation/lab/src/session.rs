//! A deliberately bounded proof document. This is not the production CAD format.
use crate::geometry::Definition;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub definition: Definition,
    #[serde(default = "sample_enabled")]
    pub sample_geometry: bool,
}
fn sample_enabled() -> bool {
    true
}
impl Default for Document {
    fn default() -> Self {
        Self {
            format: "multicad-foundation-proof".into(),
            version: 1,
            name: "Shared sketch proof".into(),
            definition: Definition::default(),
            sample_geometry: true,
        }
    }
}
impl Document {
    pub fn validate(&self) -> Result<(), String> {
        if self.format != "multicad-foundation-proof" || self.version != 1 {
            return Err(
                "Unsupported proof document format/version; original file preserved".into(),
            );
        }
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err("Name must contain 1–256 bytes".into());
        }
        let d = &self.definition;
        if ![d.width, d.depth, d.heights[0], d.heights[1], d.cut_width]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.01 && *v <= 100_000.0)
            || d.cut_width >= d.width
        {
            return Err("Invalid model dimensions".into());
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 65536 {
            return Err("Proof document exceeds 64 KiB".into());
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[derive(Default)]
pub struct Session {
    pub current: Document,
    pub saved: Option<Document>,
    undo: Vec<Document>,
    redo: Vec<Document>,
    pub revision: u64,
}
impl Session {
    pub fn commit(&mut self, base: u64, next: Document) -> Result<(), String> {
        if base != self.revision {
            return Err("Stale edit rejected".into());
        }
        next.validate()?;
        if next != self.current {
            self.undo.push(std::mem::replace(&mut self.current, next));
            self.redo.clear();
            self.revision += 1;
        }
        Ok(())
    }
    pub fn undo_target(&self) -> Option<Document> {
        self.undo.last().cloned()
    }
    pub fn redo_target(&self) -> Option<Document> {
        self.redo.last().cloned()
    }
    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo
                .push(std::mem::replace(&mut self.current, previous));
            self.revision += 1;
        }
    }
    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.current, next));
            self.revision += 1;
        }
    }
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.current)
    }
    pub fn loaded(&mut self, value: Document) {
        self.current = value.clone();
        self.saved = Some(value);
        self.undo.clear();
        self.redo.clear();
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revisioned_edits_undo_redo_and_failed_load_preserve_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proof.json");
        let mut s = Session::default();
        let mut edit = s.current.clone();
        edit.name = "Edited".into();
        s.commit(0, edit.clone()).unwrap();
        assert!(s.commit(0, edit).is_err());
        s.current.save(&path).unwrap();
        s.saved = Some(s.current.clone());
        assert!(!s.dirty());
        s.undo();
        assert!(s.dirty());
        s.redo();
        assert!(!s.dirty());
        assert_eq!(Document::load(&path).unwrap(), s.current);
        std::fs::write(&path, b"{\"version\":999}").unwrap();
        let before = s.current.clone();
        assert!(Document::load(&path).is_err());
        assert_eq!(s.current, before);
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"version\":999}");
    }
    #[test]
    fn unsafe_format_dimension_and_oversized_inputs_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        let mut d = Document {
            version: 2,
            ..Default::default()
        };
        assert!(d.save(&path).is_err());
        assert!(!path.exists());
        d.version = 1;
        d.definition.width = f64::INFINITY;
        assert!(d.validate().is_err());
        std::fs::write(&path, vec![b' '; 65537]).unwrap();
        assert!(Document::load(&path).is_err());
    }
}
