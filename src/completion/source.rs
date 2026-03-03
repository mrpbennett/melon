use super::spec::{CandidateKind, CompletionCandidate};

/// Trait for completion data sources.
pub trait CompletionSource {
    /// Generate completion candidates for the given context.
    fn candidates(&self, context: &CompletionContext) -> Vec<CompletionCandidate>;
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
pub struct PathSource;

impl CompletionSource for PathSource {
    fn candidates(&self, context: &CompletionContext) -> Vec<CompletionCandidate> {
        let base_path = if context.partial.is_empty() {
            ".".to_string()
        } else if context.partial.contains('/') {
            // Get the directory part
            let last_slash = context.partial.rfind('/').unwrap();
            if last_slash == 0 {
                "/".to_string()
            } else {
                context.partial[..last_slash].to_string()
            }
        } else {
            ".".to_string()
        };

        let prefix = if context.partial.contains('/') {
            let last_slash = context.partial.rfind('/').unwrap();
            &context.partial[..=last_slash]
        } else {
            ""
        };

        let file_prefix = if context.partial.contains('/') {
            let last_slash = context.partial.rfind('/').unwrap();
            &context.partial[last_slash + 1..]
        } else {
            &context.partial
        };

        let mut candidates = Vec::new();
        let dir = std::path::Path::new(&base_path);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files unless the partial starts with a dot
                if name.starts_with('.') && !file_prefix.starts_with('.') {
                    continue;
                }
                if !file_prefix.is_empty() && !name.starts_with(file_prefix) {
                    continue;
                }
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                let display_name = format!("{prefix}{name}{}", if is_dir { "/" } else { "" });
                candidates.push(CompletionCandidate {
                    name: display_name,
                    description: if is_dir { Some("Directory".into()) } else { None },
                    kind: if is_dir { CandidateKind::Folder } else { CandidateKind::File },
                });
            }
        }
        candidates.sort_by(|a, b| a.name.cmp(&b.name));
        candidates
    }
}
