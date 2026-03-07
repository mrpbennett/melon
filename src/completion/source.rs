use super::spec::{CandidateKind, CompletionCandidate, DEFAULT_CANDIDATE_PRIORITY};
use std::path::{Path, PathBuf};

/// Trait for completion data sources.
pub trait CompletionSource {
    /// Generate completion candidates for the given context.
    fn candidates(&mut self, context: &CompletionContext) -> Vec<CompletionCandidate>;
}

/// Context for a completion request.
#[derive(Debug, Clone)]
pub struct CompletionContext {
    /// The command being typed (e.g., "git").
    pub command: String,
    /// Subcommand chain (e.g., ["remote", "add"]).
    pub subcommands: Vec<String>,
    /// The partial text being completed.
    pub partial: String,
    /// Whether we're completing an option value.
    pub completing_option_arg: bool,
    /// Current working directory.
    pub cwd: String,
}

/// Completion source for filesystem paths.
#[derive(Default)]
pub struct PathSource {
    cached_dir: Option<CachedDir>,
}

struct CachedDir {
    base_path: PathBuf,
    entries: Vec<CachedDirEntry>,
}

struct CachedDirEntry {
    name: String,
    is_dir: bool,
}

impl CompletionSource for PathSource {
    fn candidates(&mut self, context: &CompletionContext) -> Vec<CompletionCandidate> {
        let (base_path, prefix, file_prefix) = self.resolve_context(context);
        self.ensure_cached_dir(&base_path);
        let mut candidates = Vec::new();
        let Some(cache) = &self.cached_dir else {
            return candidates;
        };

        for entry in &cache.entries {
            if entry.name.starts_with('.') && !file_prefix.starts_with('.') {
                continue;
            }
            if !file_prefix.is_empty() && !entry.name.starts_with(file_prefix) {
                continue;
            }

            let display_name = format!(
                "{prefix}{}{}",
                entry.name,
                if entry.is_dir { "/" } else { "" }
            );
            candidates.push(CompletionCandidate {
                name: display_name,
                insert_value: None,
                display_name: None,
                description: if entry.is_dir {
                    Some("Directory".into())
                } else {
                    None
                },
                icon: None,
                priority: DEFAULT_CANDIDATE_PRIORITY,
                kind: if entry.is_dir {
                    CandidateKind::Folder
                } else {
                    CandidateKind::File
                },
            });
        }

        candidates
    }
}

impl PathSource {
    fn resolve_context<'a>(&self, context: &'a CompletionContext) -> (PathBuf, &'a str, &'a str) {
        let cwd = Path::new(&context.cwd);

        if context.partial.is_empty() {
            return (cwd.to_path_buf(), "", "");
        }

        if let Some(last_slash) = context.partial.rfind('/') {
            let prefix = &context.partial[..=last_slash];
            let file_prefix = &context.partial[last_slash + 1..];
            let raw_base = if last_slash == 0 {
                PathBuf::from("/")
            } else {
                PathBuf::from(&context.partial[..last_slash])
            };
            let base_path = if raw_base.is_absolute() {
                raw_base
            } else {
                cwd.join(raw_base)
            };
            return (base_path, prefix, file_prefix);
        }

        (cwd.to_path_buf(), "", context.partial.as_str())
    }

    fn ensure_cached_dir(&mut self, base_path: &Path) {
        if self
            .cached_dir
            .as_ref()
            .is_some_and(|cache| cache.base_path == base_path)
        {
            return;
        }

        let entries = match std::fs::read_dir(base_path) {
            Ok(entries) => {
                let mut cached = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                    cached.push(CachedDirEntry { name, is_dir });
                }
                cached.sort_by(|a, b| a.name.cmp(&b.name));
                cached
            }
            Err(_) => Vec::new(),
        };

        self.cached_dir = Some(CachedDir {
            base_path: base_path.to_path_buf(),
            entries,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_source_reuses_cached_listing_for_same_base_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), "a").unwrap();

        let partial = format!("{}/", dir.path().display());
        let context = CompletionContext {
            command: "test".into(),
            subcommands: vec![],
            partial: partial.clone(),
            completing_option_arg: false,
            cwd: ".".into(),
        };

        let mut source = PathSource::default();
        let first = source.candidates(&context);
        assert!(first
            .iter()
            .any(|candidate| candidate.name.ends_with("alpha.txt")));

        std::fs::write(dir.path().join("beta.txt"), "b").unwrap();
        let second = source.candidates(&context);
        assert!(!second
            .iter()
            .any(|candidate| candidate.name.ends_with("beta.txt")));
    }

    #[test]
    fn test_path_source_invalidates_cache_when_base_path_changes() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("first.txt"), "a").unwrap();

        let mut source = PathSource::default();
        let root_context = CompletionContext {
            command: "test".into(),
            subcommands: vec![],
            partial: format!("{}/", dir.path().display()),
            completing_option_arg: false,
            cwd: ".".into(),
        };
        let _ = source.candidates(&root_context);

        std::fs::write(nested.join("second.txt"), "b").unwrap();
        let nested_context = CompletionContext {
            command: "test".into(),
            subcommands: vec![],
            partial: format!("{}/", nested.display()),
            completing_option_arg: false,
            cwd: ".".into(),
        };
        let nested_results = source.candidates(&nested_context);

        assert!(nested_results
            .iter()
            .any(|candidate| candidate.name.ends_with("first.txt")));
        assert!(nested_results
            .iter()
            .any(|candidate| candidate.name.ends_with("second.txt")));
    }
}
