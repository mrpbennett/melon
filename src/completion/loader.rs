use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use std::collections::HashMap;
use std::path::Path;

use super::spec::Spec;

/// Embedded specs compiled into the binary.
static EMBEDDED_SPECS: Dir = include_dir!("$CARGO_MANIFEST_DIR/specs");

/// In-memory index of loaded completion specs, keyed by command name.
pub struct SpecStore {
    specs: HashMap<String, Spec>,
}

impl Default for SpecStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SpecStore {
    pub fn new() -> Self {
        Self {
            specs: HashMap::new(),
        }
    }

    /// Load all JSON spec files from a directory.
    pub fn load_dir(&mut self, dir: &Path) -> Result<usize> {
        let mut count = 0;
        if !dir.exists() {
            return Ok(0);
        }
        for entry in std::fs::read_dir(dir).context("failed to read specs directory")? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                match self.load_file(&path) {
                    Ok(_) => count += 1,
                    Err(e) => {
                        tracing::warn!(?path, "failed to load spec: {e}");
                    }
                }
            }
        }
        tracing::info!(count, "loaded specs from directory");
        Ok(count)
    }

    /// Load a single JSON spec file.
    pub fn load_file(&mut self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let spec: Spec = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        self.insert(spec);
        Ok(())
    }

    /// Load specs from embedded data (include_dir at compile time).
    pub fn load_embedded(&mut self, data: &[(&str, &str)]) -> usize {
        let mut count = 0;
        for (name, content) in data {
            match serde_json::from_str::<Spec>(content) {
                Ok(spec) => {
                    self.insert(spec);
                    count += 1;
                }
                Err(e) => {
                    tracing::warn!(name, "failed to parse embedded spec: {e}");
                }
            }
        }
        count
    }

    /// Load all specs embedded in the binary at compile time.
    pub fn load_builtin(&mut self) -> usize {
        let mut count = 0;
        for file in EMBEDDED_SPECS.files() {
            let name = file.path().display().to_string();
            if let Some(content) = file.contents_utf8() {
                match serde_json::from_str::<Spec>(content) {
                    Ok(spec) => {
                        self.insert(spec);
                        count += 1;
                    }
                    Err(e) => {
                        tracing::warn!(name, "failed to parse builtin spec: {e}");
                    }
                }
            }
        }
        count
    }

    fn insert(&mut self, spec: Spec) {
        for name in spec.name.iter() {
            self.specs.insert(name.to_string(), spec.clone());
        }
    }

    /// Look up a spec by command name.
    pub fn get(&self, command: &str) -> Option<&Spec> {
        self.specs.get(command)
    }

    /// Number of unique command names (including aliases).
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// All known command names.
    pub fn iter_commands(&self) -> impl Iterator<Item = &str> + '_ {
        self.specs.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_embedded() {
        let mut store = SpecStore::new();
        let spec_json = r#"{
            "name": "cargo",
            "description": "Rust package manager",
            "subcommands": [
                {"name": "build", "description": "Compile the current package"},
                {"name": "test", "description": "Run the tests"}
            ]
        }"#;
        let count = store.load_embedded(&[("cargo.json", spec_json)]);
        assert_eq!(count, 1);
        assert!(store.get("cargo").is_some());
        assert_eq!(store.get("cargo").unwrap().subcommands.len(), 2);
    }

    #[test]
    fn test_alias_lookup() {
        let mut store = SpecStore::new();
        let spec_json = r#"{
            "name": ["python3", "python"],
            "description": "Python interpreter"
        }"#;
        store.load_embedded(&[("python.json", spec_json)]);
        assert!(store.get("python3").is_some());
        assert!(store.get("python").is_some());
    }
}
