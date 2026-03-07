use super::loader::SpecStore;
use super::source::{CompletionContext, CompletionSource, PathSource};
use super::spec::*;
use crate::input::parser::{self, ParsedLine};

/// The main completion engine. Given the current command line text, produces
/// a list of completion candidates by walking the spec tree.
pub struct CompletionEngine {
    store: SpecStore,
    command_candidates: Vec<CompletionCandidate>,
    path_source: PathSource,
    cached_context: Option<CachedCompletion>,
}

#[derive(Debug, Clone)]
pub struct CompletionResult {
    pub partial: String,
    pub candidates: Vec<CompletionCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompletionCacheKey {
    command: Option<String>,
    subcommands: Vec<String>,
    mode: CompletionModeKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CompletionModeKey {
    CommandName,
    Options,
    OptionArg { option_name: String },
    Positional { index: usize, include_options: bool },
}

#[derive(Clone)]
struct CachedCompletion {
    key: CompletionCacheKey,
    candidates: Vec<CompletionCandidate>,
}

struct ResolvedContext<'a> {
    key: CompletionCacheKey,
    kind: ResolvedKind<'a>,
    path_context: Option<CompletionContext>,
}

enum ResolvedKind<'a> {
    CommandName,
    Options(SpecNode<'a>),
    OptionArg {
        node: SpecNode<'a>,
        option_name: &'a str,
    },
    Positional {
        node: SpecNode<'a>,
        positional_index: usize,
        include_options: bool,
    },
}

impl CompletionEngine {
    pub fn new(store: SpecStore) -> Self {
        let mut command_candidates: Vec<CompletionCandidate> = store
            .iter_commands()
            .filter_map(|command| {
                store.get(command).map(|spec| CompletionCandidate {
                    name: command.to_string(),
                    description: spec.description.clone(),
                    kind: CandidateKind::Subcommand,
                })
            })
            .collect();
        command_candidates.sort_by(|a, b| a.name.cmp(&b.name));

        Self {
            store,
            command_candidates,
            path_source: PathSource::default(),
            cached_context: None,
        }
    }

    pub fn store(&self) -> &SpecStore {
        &self.store
    }

    /// Generate candidates for the current input line.
    pub fn complete(&mut self, input: &str) -> CompletionResult {
        let parsed = parser::parse_completion_input(input);
        let candidates = self.complete_parsed(&parsed);
        CompletionResult {
            partial: parsed.partial,
            candidates,
        }
    }

    fn complete_parsed(&mut self, parsed: &ParsedLine) -> Vec<CompletionCandidate> {
        let Some(resolved) = Self::resolve_context(&self.store, parsed) else {
            self.cached_context = None;
            return vec![];
        };

        let mut candidates = if self
            .cached_context
            .as_ref()
            .is_some_and(|cached| cached.key == resolved.key)
        {
            self.cached_context
                .as_ref()
                .map(|cached| cached.candidates.clone())
                .unwrap_or_default()
        } else {
            let built = Self::build_base_candidates(&self.command_candidates, &resolved);
            self.cached_context = Some(CachedCompletion {
                key: resolved.key.clone(),
                candidates: built.clone(),
            });
            built
        };

        if let Some(path_context) = &resolved.path_context {
            candidates.extend(self.path_source.candidates(path_context));
        }

        candidates
    }

    fn resolve_context<'a>(
        store: &'a SpecStore,
        parsed: &'a ParsedLine,
    ) -> Option<ResolvedContext<'a>> {
        if parsed.tokens.is_empty() {
            return Some(ResolvedContext {
                key: CompletionCacheKey {
                    command: None,
                    subcommands: vec![],
                    mode: CompletionModeKey::CommandName,
                },
                kind: ResolvedKind::CommandName,
                path_context: None,
            });
        }

        let command = parsed.tokens[0].text.as_str();
        let spec = store.get(command)?;
        let (subcommand_node, remaining_tokens, subcommand_chain) =
            Self::walk_subcommands(spec, &parsed.tokens[1..]);
        let partial = parsed.partial.as_str();

        if partial.starts_with('-') {
            return Some(ResolvedContext {
                key: CompletionCacheKey {
                    command: Some(command.to_string()),
                    subcommands: subcommand_chain
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                    mode: CompletionModeKey::Options,
                },
                kind: ResolvedKind::Options(subcommand_node),
                path_context: None,
            });
        }

        if let Some(option_token) = remaining_tokens.last().filter(|token| {
            token.text.starts_with('-') && Self::option_takes_arg(subcommand_node, &token.text)
        }) {
            return Some(ResolvedContext {
                key: CompletionCacheKey {
                    command: Some(command.to_string()),
                    subcommands: subcommand_chain
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                    mode: CompletionModeKey::OptionArg {
                        option_name: option_token.text.clone(),
                    },
                },
                kind: ResolvedKind::OptionArg {
                    node: subcommand_node,
                    option_name: option_token.text.as_str(),
                },
                path_context: Self::option_arg_path_context(
                    subcommand_node,
                    option_token.text.as_str(),
                    partial,
                    command,
                    &subcommand_chain,
                ),
            });
        }

        let positional_index = remaining_tokens
            .iter()
            .filter(|token| !token.text.starts_with('-'))
            .count();
        let include_options = partial.is_empty();

        Some(ResolvedContext {
            key: CompletionCacheKey {
                command: Some(command.to_string()),
                subcommands: subcommand_chain
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                mode: CompletionModeKey::Positional {
                    index: positional_index,
                    include_options,
                },
            },
            kind: ResolvedKind::Positional {
                node: subcommand_node,
                positional_index,
                include_options,
            },
            path_context: Self::positional_path_context(
                subcommand_node,
                positional_index,
                partial,
                command,
                &subcommand_chain,
            ),
        })
    }

    fn build_base_candidates(
        command_candidates: &[CompletionCandidate],
        resolved: &ResolvedContext<'_>,
    ) -> Vec<CompletionCandidate> {
        match resolved.kind {
            ResolvedKind::CommandName => command_candidates.to_vec(),
            ResolvedKind::Options(node) => Self::complete_options(node),
            ResolvedKind::OptionArg { node, option_name } => {
                Self::complete_option_arg(node, option_name)
            }
            ResolvedKind::Positional {
                node,
                positional_index,
                include_options,
            } => Self::complete_positional(node, positional_index, include_options),
        }
    }

    fn walk_subcommands<'a>(
        spec: &'a Spec,
        tokens: &'a [parser::Token],
    ) -> (SpecNode<'a>, Vec<&'a parser::Token>, Vec<&'a str>) {
        let mut current = SpecNode::Root(spec);
        let mut remaining = Vec::new();
        let mut matched_subcommands = Vec::new();

        for token in tokens {
            if token.text.starts_with('-') {
                remaining.push(token);
                continue;
            }

            let subcommands = match &current {
                SpecNode::Root(spec) => &spec.subcommands,
                SpecNode::Sub(subcommand) => &subcommand.subcommands,
            };

            if let Some(subcommand) = subcommands
                .iter()
                .find(|subcommand| subcommand.name.contains(&token.text))
            {
                current = SpecNode::Sub(subcommand);
                matched_subcommands.push(token.text.as_str());
            } else {
                remaining.push(token);
            }
        }

        (current, remaining, matched_subcommands)
    }

    fn complete_options(node: SpecNode<'_>) -> Vec<CompletionCandidate> {
        let mut candidates = Vec::new();
        for opt in Self::get_options(node) {
            if opt.hidden {
                continue;
            }
            for name in opt.name.iter() {
                candidates.push(CompletionCandidate {
                    name: name.to_string(),
                    description: opt.description.clone(),
                    kind: CandidateKind::Option,
                });
            }
        }
        candidates.sort_by(|a, b| a.name.cmp(&b.name));
        candidates
    }

    fn option_takes_arg(node: SpecNode<'_>, option_name: &str) -> bool {
        Self::get_options(node)
            .iter()
            .any(|opt| opt.name.contains(option_name) && !opt.args.is_empty())
    }

    fn complete_option_arg(node: SpecNode<'_>, option_name: &str) -> Vec<CompletionCandidate> {
        for opt in Self::get_options(node) {
            if !opt.name.contains(option_name) {
                continue;
            }

            let Some(arg) = opt.args.first() else {
                return vec![];
            };

            let mut candidates = Vec::new();
            for suggestion in &arg.suggestions {
                let (name, desc) = match suggestion {
                    SuggestionOrString::String(value) => (value.clone(), None),
                    SuggestionOrString::Suggestion(value) => {
                        (value.name.primary().to_string(), value.description.clone())
                    }
                };
                candidates.push(CompletionCandidate {
                    name,
                    description: desc,
                    kind: CandidateKind::Argument,
                });
            }
            return candidates;
        }

        vec![]
    }

    fn complete_positional(
        node: SpecNode<'_>,
        positional_index: usize,
        include_options: bool,
    ) -> Vec<CompletionCandidate> {
        let mut candidates = Vec::new();

        for subcommand in Self::get_subcommands(node) {
            if subcommand.hidden {
                continue;
            }
            for name in subcommand.name.iter() {
                candidates.push(CompletionCandidate {
                    name: name.to_string(),
                    description: subcommand.description.clone(),
                    kind: CandidateKind::Subcommand,
                });
            }
        }

        let arg_to_complete = Self::get_args(node)
            .enumerate()
            .find(|(index, arg)| {
                *index == positional_index || (arg.is_variadic && *index < positional_index)
            })
            .map(|(_, arg)| arg);

        if let Some(arg) = arg_to_complete {
            for suggestion in &arg.suggestions {
                match suggestion {
                    SuggestionOrString::String(value) => {
                        candidates.push(CompletionCandidate {
                            name: value.clone(),
                            description: None,
                            kind: CandidateKind::Argument,
                        });
                    }
                    SuggestionOrString::Suggestion(value) => {
                        if value.hidden {
                            continue;
                        }
                        for name in value.name.iter() {
                            candidates.push(CompletionCandidate {
                                name: name.to_string(),
                                description: value.description.clone(),
                                kind: CandidateKind::Argument,
                            });
                        }
                    }
                }
            }
        }

        if include_options {
            for opt in Self::get_options(node) {
                if opt.hidden {
                    continue;
                }
                candidates.push(CompletionCandidate {
                    name: opt.name.preferred().to_string(),
                    description: opt.description.clone(),
                    kind: CandidateKind::Option,
                });
            }
        }

        candidates
    }

    fn option_arg_path_context(
        node: SpecNode<'_>,
        option_name: &str,
        partial: &str,
        command: &str,
        subcommands: &[&str],
    ) -> Option<CompletionContext> {
        for opt in Self::get_options(node) {
            if opt.name.contains(option_name)
                && opt.args.first().is_some_and(Self::arg_uses_path_templates)
            {
                return Some(Self::make_path_context(command, subcommands, partial, true));
            }
        }
        None
    }

    fn positional_path_context(
        node: SpecNode<'_>,
        positional_index: usize,
        partial: &str,
        command: &str,
        subcommands: &[&str],
    ) -> Option<CompletionContext> {
        Self::get_args(node)
            .enumerate()
            .find(|(index, arg)| {
                *index == positional_index || (arg.is_variadic && *index < positional_index)
            })
            .map(|(_, arg)| arg)
            .filter(|arg| Self::arg_uses_path_templates(arg))
            .map(|_| Self::make_path_context(command, subcommands, partial, false))
    }

    fn make_path_context(
        command: &str,
        subcommands: &[&str],
        partial: &str,
        completing_option_arg: bool,
    ) -> CompletionContext {
        CompletionContext {
            command: command.to_string(),
            subcommands: subcommands
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            partial: partial.to_string(),
            completing_option_arg,
            cwd: ".".to_string(),
        }
    }

    fn arg_uses_path_templates(arg: &Arg) -> bool {
        match &arg.template {
            Some(Template::Single(kind)) => {
                matches!(kind, TemplateKind::Filepaths | TemplateKind::Folders)
            }
            Some(Template::Multiple(kinds)) => kinds
                .iter()
                .any(|kind| matches!(kind, TemplateKind::Filepaths | TemplateKind::Folders)),
            None => false,
        }
    }

    fn get_subcommands<'a>(node: SpecNode<'a>) -> &'a [Subcommand] {
        match node {
            SpecNode::Root(spec) => &spec.subcommands,
            SpecNode::Sub(subcommand) => &subcommand.subcommands,
        }
    }

    fn get_options<'a>(node: SpecNode<'a>) -> &'a [Opt] {
        match node {
            SpecNode::Root(spec) => &spec.options,
            SpecNode::Sub(subcommand) => &subcommand.options,
        }
    }

    fn get_args<'a>(node: SpecNode<'a>) -> ArgOrArgsIter<'a> {
        match node {
            SpecNode::Root(spec) => spec.args.iter(),
            SpecNode::Sub(subcommand) => subcommand.args.iter(),
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
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git com").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"compare"));
    }

    #[test]
    fn test_complete_subcommands_with_matcher() {
        use crate::completion::matcher::FuzzyMatcher;

        let mut engine = CompletionEngine::new(test_store());
        let completion = engine.complete("git com");
        let mut matcher = FuzzyMatcher::new();
        let scored = matcher.filter(&completion.partial, completion.candidates);
        let names: Vec<&str> = scored.iter().map(|s| s.candidate.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"compare"));
        assert!(!names.contains(&"checkout"));
    }

    #[test]
    fn test_complete_options() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git commit -").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"-m"));
        assert!(names.contains(&"--message"));
    }

    #[test]
    fn test_no_completions_after_non_template_arg() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git clone ").candidates;
        let file_candidates: Vec<&str> = candidates
            .iter()
            .filter(|c| matches!(c.kind, CandidateKind::File | CandidateKind::Folder))
            .map(|c| c.name.as_str())
            .collect();
        assert!(
            file_candidates.is_empty(),
            "should not offer filesystem completions at position 0 of clone"
        );
    }

    #[test]
    fn test_complete_all_subcommands() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git ").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"clone"));
        assert!(names.contains(&"checkout"));
    }
}
