use super::loader::SpecStore;
use super::source::{CompletionContext, CompletionSource, PathSource};
use super::spec::*;
use crate::input::parser;

/// The main completion engine. Given the current command line text, produces
/// a list of completion candidates by walking the spec tree.
pub struct CompletionEngine {
    store: SpecStore,
}

impl CompletionEngine {
    pub fn new(store: SpecStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &SpecStore {
        &self.store
    }

    /// Generate candidates for the current input line.
    pub fn complete(&self, input: &str) -> Vec<CompletionCandidate> {
        let (tokens, partial) = parser::split_partial(input);

        if tokens.is_empty() {
            // Completing the command name itself
            return self.complete_command_name(&partial);
        }

        let command = &tokens[0].text;
        let spec = match self.store.get(command) {
            Some(s) => s,
            None => {
                // No spec for this command — show nothing
                return vec![];
            }
        };

        // Walk the spec tree following subcommands
        let rest = &tokens[1..];
        let (subcommand_node, remaining_tokens) = self.walk_subcommands(spec, rest);

        // Determine what to complete based on context
        let completing_option = partial.starts_with('-');

        if completing_option {
            // Complete options
            self.complete_options(subcommand_node, &partial)
        } else {
            // Check if previous token was an option that takes an arg
            let prev_is_option_with_arg = remaining_tokens.last().is_some_and(|t| {
                t.text.starts_with('-') && self.option_takes_arg(subcommand_node, &t.text)
            });

            if prev_is_option_with_arg {
                // Complete the option's argument
                self.complete_option_arg(subcommand_node, &remaining_tokens.last().unwrap().text, &partial)
            } else {
                // Complete subcommands or positional args
                let mut candidates = Vec::new();

                // Subcommands
                for sub in self.get_subcommands(subcommand_node) {
                    if !sub.hidden {
                        for name in sub.name.all() {
                            candidates.push(CompletionCandidate {
                                name: name.to_string(),
                                description: sub.description.clone(),
                                kind: CandidateKind::Subcommand,
                            });
                        }
                    }
                }

                // Positional args (if they have suggestions or templates)
                // Track which positional index we're completing based on non-option tokens already present.
                let positional_count = remaining_tokens.iter().filter(|t| !t.text.starts_with('-')).count();
                let args = self.get_args(subcommand_node);
                // Find the arg slot for the current position (last variadic arg absorbs everything beyond its index)
                let arg_to_complete = args.iter().enumerate().find(|(i, arg)| {
                    *i == positional_count || (arg.is_variadic && *i < positional_count)
                }).map(|(_, arg)| arg);

                if let Some(arg) = arg_to_complete {
                    // Static suggestions
                    for suggestion in &arg.suggestions {
                        match suggestion {
                            SuggestionOrString::String(s) => {
                                candidates.push(CompletionCandidate {
                                    name: s.clone(),
                                    description: None,
                                    kind: CandidateKind::Argument,
                                });
                            }
                            SuggestionOrString::Suggestion(s) => {
                                if !s.hidden {
                                    for name in s.name.all() {
                                        candidates.push(CompletionCandidate {
                                            name: name.to_string(),
                                            description: s.description.clone(),
                                            kind: CandidateKind::Argument,
                                        });
                                    }
                                }
                            }
                        }
                    }

                    // Template-based completions
                    if let Some(template) = &arg.template {
                        let kinds = match template {
                            Template::Single(k) => vec![k.clone()],
                            Template::Multiple(ks) => ks.clone(),
                        };
                        for kind in kinds {
                            if matches!(kind, TemplateKind::Filepaths | TemplateKind::Folders) {
                                let ctx = CompletionContext {
                                    command: command.clone(),
                                    subcommands: vec![],
                                    partial: partial.clone(),
                                    completing_option_arg: false,
                                    cwd: ".".to_string(),
                                };
                                candidates.extend(PathSource.candidates(&ctx));
                            }
                        }
                    }
                }

                // If no subcommands or arg suggestions, offer options too
                if candidates.is_empty() || partial.is_empty() {
                    // Also offer options when no partial (user just pressed tab)
                    if partial.is_empty() {
                        for opt in self.get_options(subcommand_node) {
                            if !opt.hidden {
                                // Only show the long form if available
                                let names = opt.name.all();
                                let display_name = names.iter()
                                    .find(|n| n.starts_with("--"))
                                    .or(names.first())
                                    .unwrap_or(&"");
                                candidates.push(CompletionCandidate {
                                    name: display_name.to_string(),
                                    description: opt.description.clone(),
                                    kind: CandidateKind::Option,
                                });
                            }
                        }
                    }
                }

                candidates
            }
        }
    }

    fn complete_command_name(&self, partial: &str) -> Vec<CompletionCandidate> {
        let mut candidates = Vec::new();
        for cmd in self.store.commands() {
            if partial.is_empty() || cmd.starts_with(partial) {
                if let Some(spec) = self.store.get(cmd) {
                    candidates.push(CompletionCandidate {
                        name: cmd.to_string(),
                        description: spec.description.clone(),
                        kind: CandidateKind::Subcommand,
                    });
                }
            }
        }
        candidates.sort_by(|a, b| a.name.cmp(&b.name));
        candidates.dedup_by(|a, b| a.name == b.name);
        candidates
    }

    /// Walk the spec tree following subcommand tokens.
    /// Returns the deepest matching node and remaining non-subcommand tokens.
    fn walk_subcommands<'a>(
        &'a self,
        spec: &'a Spec,
        tokens: &'a [parser::Token],
    ) -> (SpecNode<'a>, Vec<&'a parser::Token>) {
        let mut current = SpecNode::Root(spec);
        let mut remaining = Vec::new();
        let mut consumed_subcommand = false;

        for token in tokens {
            if token.text.starts_with('-') {
                remaining.push(token);
                continue;
            }

            // Try to match as a subcommand
            let subcommands = match &current {
                SpecNode::Root(s) => &s.subcommands,
                SpecNode::Sub(s) => &s.subcommands,
            };

            let mut found = false;
            for sub in subcommands {
                if sub.name.all().contains(&token.text.as_str()) {
                    current = SpecNode::Sub(sub);
                    found = true;
                    consumed_subcommand = true;
                    break;
                }
            }

            if !found {
                remaining.push(token);
            }
        }

        let _ = consumed_subcommand; // might use later for arg position tracking
        (current, remaining)
    }

    fn complete_options(&self, node: SpecNode<'_>, partial: &str) -> Vec<CompletionCandidate> {
        let mut candidates = Vec::new();
        for opt in self.get_options(node) {
            if opt.hidden {
                continue;
            }
            for name in opt.name.all() {
                if name.starts_with(partial) {
                    candidates.push(CompletionCandidate {
                        name: name.to_string(),
                        description: opt.description.clone(),
                        kind: CandidateKind::Option,
                    });
                }
            }
        }
        candidates
    }

    fn option_takes_arg(&self, node: SpecNode<'_>, option_name: &str) -> bool {
        for opt in self.get_options(node) {
            if opt.name.all().contains(&option_name) {
                return !opt.args.as_slice().is_empty();
            }
        }
        false
    }

    fn complete_option_arg(
        &self,
        node: SpecNode<'_>,
        option_name: &str,
        partial: &str,
    ) -> Vec<CompletionCandidate> {
        for opt in self.get_options(node) {
            if opt.name.all().contains(&option_name) {
                let args = opt.args.as_slice();
                if let Some(arg) = args.first() {
                    let mut candidates = Vec::new();
                    for s in &arg.suggestions {
                        let (name, desc) = match s {
                            SuggestionOrString::String(s) => (s.clone(), None),
                            SuggestionOrString::Suggestion(s) => {
                                (s.name.primary().to_string(), s.description.clone())
                            }
                        };
                        if partial.is_empty() || name.starts_with(partial) {
                            candidates.push(CompletionCandidate {
                                name,
                                description: desc,
                                kind: CandidateKind::Argument,
                            });
                        }
                    }
                    if let Some(template) = &arg.template {
                        let has_files = match template {
                            Template::Single(k) => matches!(k, TemplateKind::Filepaths | TemplateKind::Folders),
                            Template::Multiple(ks) => ks.iter().any(|k| matches!(k, TemplateKind::Filepaths | TemplateKind::Folders)),
                        };
                        if has_files {
                            let ctx = CompletionContext {
                                command: String::new(),
                                subcommands: vec![],
                                partial: partial.to_string(),
                                completing_option_arg: true,
                                cwd: ".".to_string(),
                            };
                            candidates.extend(PathSource.candidates(&ctx));
                        }
                    }
                    return candidates;
                }
            }
        }
        vec![]
    }

    fn get_subcommands<'a>(&self, node: SpecNode<'a>) -> &'a [Subcommand] {
        match node {
            SpecNode::Root(s) => &s.subcommands,
            SpecNode::Sub(s) => &s.subcommands,
        }
    }

    fn get_options<'a>(&self, node: SpecNode<'a>) -> &'a [Opt] {
        match node {
            SpecNode::Root(s) => &s.options,
            SpecNode::Sub(s) => &s.options,
        }
    }

    fn get_args<'a>(&self, node: SpecNode<'a>) -> Vec<&'a Arg> {
        match node {
            SpecNode::Root(s) => s.args.as_slice(),
            SpecNode::Sub(s) => s.args.as_slice(),
        }
    }
}

#[derive(Clone, Copy)]
enum SpecNode<'a> {
    Root(&'a Spec),
    Sub(&'a Subcommand),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SpecStore {
        let mut store = SpecStore::new();
        let git_spec = r#"{
            "name": "git",
            "description": "Version control",
            "subcommands": [
                {"name": "commit", "description": "Record changes",
                 "options": [
                    {"name": ["-m", "--message"], "description": "Commit message",
                     "args": {"name": "message"}}
                 ]},
                {"name": "compare", "description": "Compare branches"},
                {"name": "clone", "description": "Clone a repository"},
                {"name": "checkout", "description": "Switch branches",
                 "options": [
                    {"name": "-b", "description": "Create and checkout new branch"}
                 ]},
                {"name": "add", "description": "Add files",
                 "args": {"template": "filepaths", "isVariadic": true}}
            ],
            "options": [
                {"name": "--version", "description": "Print version"},
                {"name": "--help", "description": "Show help"}
            ]
        }"#;
        store.load_embedded(&[("git.json", git_spec)]);
        store
    }

    #[test]
    fn test_complete_subcommands() {
        let engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git com");
        // Engine returns all subcommands; fuzzy matcher filters in the full pipeline.
        // But the engine should at least return commit and compare.
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"compare"));
    }

    #[test]
    fn test_complete_subcommands_with_matcher() {
        use crate::completion::matcher::FuzzyMatcher;
        let engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git com");
        let mut matcher = FuzzyMatcher::new();
        let scored = matcher.filter("com", candidates);
        let names: Vec<&str> = scored.iter().map(|s| s.candidate.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"compare"));
        assert!(!names.contains(&"checkout"));
    }

    #[test]
    fn test_complete_options() {
        let engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git commit -");
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"-m"));
        assert!(names.contains(&"--message"));
    }

    #[test]
    fn test_no_completions_after_non_template_arg() {
        // `git clone ` — positional 0 is "repository" with no template/suggestions.
        // Should return empty (no OS directories).
        let engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git clone ");
        let file_candidates: Vec<&str> = candidates.iter()
            .filter(|c| matches!(c.kind, CandidateKind::File | CandidateKind::Folder))
            .map(|c| c.name.as_str())
            .collect();
        assert!(file_candidates.is_empty(), "should not offer filesystem completions at position 0 of clone");
    }

    #[test]
    fn test_complete_all_subcommands() {
        let engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git ");
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"clone"));
        assert!(names.contains(&"checkout"));
    }
}
