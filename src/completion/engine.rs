use super::generator::{suggestion_candidates, GeneratorContext, GeneratorSource};
use super::loader::SpecStore;
use super::source::{CompletionContext, CompletionSource, PathSource};
use super::spec::*;
use crate::input::parser::{self, ParsedLine};
use std::collections::HashSet;

/// The main completion engine. Given the current command line text, produces
/// a list of completion candidates by walking the spec tree.
pub struct CompletionEngine {
    store: SpecStore,
    command_candidates: Vec<CompletionCandidate>,
    path_source: PathSource,
    generator_source: GeneratorSource,
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
    Options {
        used_options: Vec<String>,
    },
    OptionArg {
        option_name: String,
    },
    Positional {
        index: usize,
        include_options: bool,
        used_options: Vec<String>,
    },
}

#[derive(Clone)]
struct CachedCompletion {
    key: CompletionCacheKey,
    candidates: Vec<CompletionCandidate>,
}

struct ResolvedContext<'a> {
    key: CompletionCacheKey,
    kind: ResolvedKind<'a>,
    option_scope: Option<OptionScope<'a>>,
    path_context: Option<CompletionContext>,
    generator_context: Option<ResolvedGeneratorContext<'a>>,
}

enum ResolvedKind<'a> {
    CommandName,
    Options,
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

struct ResolvedGeneratorContext<'a> {
    arg: &'a Arg,
    context: GeneratorContext,
}

struct CandidateMetadata<'a> {
    names: &'a StringOrArray,
    description: &'a Option<String>,
    display_name: &'a Option<String>,
    icon: &'a Option<String>,
    insert_value: &'a Option<String>,
    priority: Option<i32>,
}

struct OptionScope<'a> {
    all: Vec<&'a Opt>,
    available: Vec<&'a Opt>,
    used_options: Vec<String>,
}

struct OptionUsage<'a> {
    used_preferred_names: HashSet<String>,
    used_aliases: HashSet<String>,
    used_options: Vec<&'a Opt>,
}

impl CompletionEngine {
    pub fn new(store: SpecStore) -> Self {
        let mut command_candidates: Vec<CompletionCandidate> = store
            .iter_commands()
            .filter_map(|command| {
                store.get(command).map(|spec| {
                    Self::named_candidate(
                        command,
                        CandidateMetadata {
                            names: &spec.name,
                            description: &spec.description,
                            display_name: &spec.display_name,
                            icon: &spec.icon,
                            insert_value: &spec.insert_value,
                            priority: spec.priority,
                        },
                        CandidateKind::Subcommand,
                    )
                })
            })
            .collect();
        command_candidates.sort_by(|a, b| a.name.cmp(&b.name));

        Self {
            store,
            command_candidates,
            path_source: PathSource::default(),
            generator_source: GeneratorSource::default(),
            cached_context: None,
        }
    }

    pub fn store(&self) -> &SpecStore {
        &self.store
    }

    /// Generate candidates for the current input line.
    pub fn complete(&mut self, input: &str, cwd: &str) -> CompletionResult {
        let parsed = parser::parse_completion_input(input);
        let candidates = self.complete_parsed(&parsed, cwd);
        CompletionResult {
            partial: parsed.partial,
            candidates,
        }
    }

    fn complete_parsed(&mut self, parsed: &ParsedLine, cwd: &str) -> Vec<CompletionCandidate> {
        let Some(resolved) = Self::resolve_context(&self.store, parsed, cwd) else {
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

        if let Some(generator_context) = &resolved.generator_context {
            candidates.extend(
                self.generator_source
                    .candidates(generator_context.arg, &generator_context.context),
            );
        }

        candidates
    }

    fn resolve_context<'a>(
        store: &'a SpecStore,
        parsed: &'a ParsedLine,
        cwd: &str,
    ) -> Option<ResolvedContext<'a>> {
        if parsed.tokens.is_empty() {
            return Some(ResolvedContext {
                key: CompletionCacheKey {
                    command: None,
                    subcommands: vec![],
                    mode: CompletionModeKey::CommandName,
                },
                kind: ResolvedKind::CommandName,
                option_scope: None,
                path_context: None,
                generator_context: None,
            });
        }

        let command = parsed.tokens[0].text.as_str();
        let spec = store.get(command)?;
        let (subcommand_node, remaining_tokens, subcommand_chain, subcommand_defs) =
            Self::walk_subcommands(spec, &parsed.tokens[1..]);
        let option_scope = Self::option_scope(spec, &subcommand_defs, &remaining_tokens);
        let partial = parsed.partial.as_str();

        if partial.starts_with('-') {
            return Some(ResolvedContext {
                key: CompletionCacheKey {
                    command: Some(command.to_string()),
                    subcommands: subcommand_chain
                        .iter()
                        .map(|value| (*value).to_string())
                        .collect(),
                    mode: CompletionModeKey::Options {
                        used_options: option_scope.used_options.clone(),
                    },
                },
                kind: ResolvedKind::Options,
                option_scope: Some(option_scope),
                path_context: None,
                generator_context: None,
            });
        }

        if let Some(option_token) = remaining_tokens.last().filter(|token| {
            token.text.starts_with('-') && Self::option_takes_arg(&option_scope, &token.text)
        }) {
            let option_arg = Self::option_arg(&option_scope, option_token.text.as_str());
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
                option_scope: Some(option_scope),
                path_context: option_arg
                    .filter(|arg| Self::arg_uses_path_templates(arg))
                    .map(|_| {
                        Self::make_path_context(command, &subcommand_chain, partial, true, cwd)
                    }),
                generator_context: option_arg.and_then(|arg| {
                    Self::make_generator_context(arg, parsed, command, &subcommand_chain, cwd, true)
                }),
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
                    used_options: if include_options {
                        option_scope.used_options.clone()
                    } else {
                        Vec::new()
                    },
                },
            },
            kind: ResolvedKind::Positional {
                node: subcommand_node,
                positional_index,
                include_options,
            },
            option_scope: Some(option_scope),
            path_context: Self::positional_arg(subcommand_node, positional_index)
                .filter(|arg| Self::arg_uses_path_templates(arg))
                .map(|_| Self::make_path_context(command, &subcommand_chain, partial, false, cwd)),
            generator_context: Self::positional_arg(subcommand_node, positional_index).and_then(
                |arg| {
                    Self::make_generator_context(
                        arg,
                        parsed,
                        command,
                        &subcommand_chain,
                        cwd,
                        false,
                    )
                },
            ),
        })
    }

    fn build_base_candidates(
        command_candidates: &[CompletionCandidate],
        resolved: &ResolvedContext<'_>,
    ) -> Vec<CompletionCandidate> {
        match resolved.kind {
            ResolvedKind::CommandName => command_candidates.to_vec(),
            ResolvedKind::Options => Self::complete_options(
                resolved
                    .option_scope
                    .as_ref()
                    .expect("option scope should exist for option completion"),
            ),
            ResolvedKind::OptionArg { node, option_name } => Self::complete_option_arg(
                node,
                resolved
                    .option_scope
                    .as_ref()
                    .expect("option scope should exist for option-arg completion"),
                option_name,
            ),
            ResolvedKind::Positional {
                node,
                positional_index,
                include_options,
            } => Self::complete_positional(
                node,
                resolved
                    .option_scope
                    .as_ref()
                    .expect("option scope should exist for positional completion"),
                positional_index,
                include_options,
            ),
        }
    }

    fn walk_subcommands<'a>(
        spec: &'a Spec,
        tokens: &'a [parser::Token],
    ) -> (
        SpecNode<'a>,
        Vec<&'a parser::Token>,
        Vec<&'a str>,
        Vec<&'a Subcommand>,
    ) {
        let mut current = SpecNode::Root(spec);
        let mut remaining = Vec::new();
        let mut matched_subcommands = Vec::new();
        let mut matched_defs = Vec::new();

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
                matched_defs.push(subcommand);
            } else {
                remaining.push(token);
            }
        }

        (current, remaining, matched_subcommands, matched_defs)
    }

    fn complete_options(scope: &OptionScope<'_>) -> Vec<CompletionCandidate> {
        let mut candidates = Vec::new();
        for opt in &scope.available {
            for name in opt.name.iter() {
                candidates.push(Self::named_candidate(
                    name,
                    CandidateMetadata {
                        names: &opt.name,
                        description: &opt.description,
                        display_name: &opt.display_name,
                        icon: &opt.icon,
                        insert_value: &opt.insert_value,
                        priority: opt.priority,
                    },
                    CandidateKind::Option,
                ));
            }
        }
        candidates.sort_by(|a, b| a.name.cmp(&b.name));
        candidates
    }

    fn option_takes_arg(scope: &OptionScope<'_>, option_name: &str) -> bool {
        scope
            .all
            .iter()
            .any(|opt| opt.name.contains(option_name) && !opt.args.is_empty())
    }

    fn complete_option_arg(
        _node: SpecNode<'_>,
        scope: &OptionScope<'_>,
        option_name: &str,
    ) -> Vec<CompletionCandidate> {
        for opt in &scope.all {
            if !opt.name.contains(option_name) {
                continue;
            }

            let Some(arg) = opt.args.first() else {
                return vec![];
            };

            let mut candidates = Vec::new();
            for suggestion in &arg.suggestions {
                candidates.extend(suggestion_candidates(suggestion, CandidateKind::Argument));
            }
            return candidates;
        }

        vec![]
    }

    fn complete_positional(
        node: SpecNode<'_>,
        scope: &OptionScope<'_>,
        positional_index: usize,
        include_options: bool,
    ) -> Vec<CompletionCandidate> {
        let mut candidates = Vec::new();

        for subcommand in Self::get_subcommands(node) {
            if subcommand.hidden {
                continue;
            }
            for name in subcommand.name.iter() {
                candidates.push(Self::named_candidate(
                    name,
                    CandidateMetadata {
                        names: &subcommand.name,
                        description: &subcommand.description,
                        display_name: &subcommand.display_name,
                        icon: &subcommand.icon,
                        insert_value: &subcommand.insert_value,
                        priority: subcommand.priority,
                    },
                    CandidateKind::Subcommand,
                ));
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
                candidates.extend(suggestion_candidates(suggestion, CandidateKind::Argument));
            }
        }

        if include_options {
            for opt in &scope.available {
                candidates.push(Self::named_candidate(
                    opt.name.preferred(),
                    CandidateMetadata {
                        names: &opt.name,
                        description: &opt.description,
                        display_name: &opt.display_name,
                        icon: &opt.icon,
                        insert_value: &opt.insert_value,
                        priority: opt.priority,
                    },
                    CandidateKind::Option,
                ));
            }
        }

        candidates
    }

    fn option_arg<'a>(scope: &OptionScope<'a>, option_name: &str) -> Option<&'a Arg> {
        scope
            .all
            .iter()
            .find(|opt| opt.name.contains(option_name))
            .and_then(|opt| opt.args.first())
    }

    fn positional_arg(node: SpecNode<'_>, positional_index: usize) -> Option<&Arg> {
        Self::get_args(node)
            .enumerate()
            .find(|(index, arg)| {
                *index == positional_index || (arg.is_variadic && *index < positional_index)
            })
            .map(|(_, arg)| arg)
    }

    fn make_path_context(
        command: &str,
        subcommands: &[&str],
        partial: &str,
        completing_option_arg: bool,
        cwd: &str,
    ) -> CompletionContext {
        CompletionContext {
            command: command.to_string(),
            subcommands: subcommands
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            partial: partial.to_string(),
            completing_option_arg,
            cwd: cwd.to_string(),
        }
    }

    fn option_scope<'a>(
        spec: &'a Spec,
        subcommand_defs: &[&'a Subcommand],
        tokens: &[&parser::Token],
    ) -> OptionScope<'a> {
        let all = Self::collect_available_options(spec, subcommand_defs);
        let usage = Self::analyze_option_usage(&all, tokens);
        let available = all
            .iter()
            .copied()
            .filter(|opt| Self::option_is_available(opt, &usage))
            .collect();
        let mut used_options: Vec<String> = usage.used_preferred_names.into_iter().collect();
        used_options.sort();

        OptionScope {
            all,
            available,
            used_options,
        }
    }

    fn collect_available_options<'a>(
        spec: &'a Spec,
        subcommand_defs: &[&'a Subcommand],
    ) -> Vec<&'a Opt> {
        let mut options = Vec::new();
        let mut seen_aliases = HashSet::new();

        if let Some(current) = subcommand_defs.last() {
            Self::push_options(&mut options, &mut seen_aliases, &current.options, false);

            for subcommand in subcommand_defs[..subcommand_defs.len() - 1].iter().rev() {
                Self::push_options(&mut options, &mut seen_aliases, &subcommand.options, true);
            }

            Self::push_options(&mut options, &mut seen_aliases, &spec.options, true);
        } else {
            Self::push_options(&mut options, &mut seen_aliases, &spec.options, false);
        }

        options
    }

    fn push_options<'a>(
        target: &mut Vec<&'a Opt>,
        seen_aliases: &mut HashSet<String>,
        options: &'a [Opt],
        persistent_only: bool,
    ) {
        for opt in options {
            if persistent_only && !opt.is_persistent {
                continue;
            }

            if opt.name.iter().any(|name| seen_aliases.contains(name)) {
                continue;
            }

            for name in opt.name.iter() {
                seen_aliases.insert(name.to_string());
            }

            target.push(opt);
        }
    }

    fn analyze_option_usage<'a>(options: &[&'a Opt], tokens: &[&parser::Token]) -> OptionUsage<'a> {
        let mut used_preferred_names = HashSet::new();
        let mut used_aliases = HashSet::new();
        let mut used_options = Vec::new();

        for token in tokens {
            if !token.text.starts_with('-') {
                continue;
            }

            let Some(opt) = options
                .iter()
                .copied()
                .find(|opt| opt.name.contains(&token.text))
            else {
                continue;
            };

            let preferred = opt.name.preferred().to_string();
            if used_preferred_names.insert(preferred) {
                used_options.push(opt);
            }

            for name in opt.name.iter() {
                used_aliases.insert(name.to_string());
            }
        }

        OptionUsage {
            used_preferred_names,
            used_aliases,
            used_options,
        }
    }

    fn option_is_available(opt: &Opt, usage: &OptionUsage<'_>) -> bool {
        if opt.hidden {
            return false;
        }

        if !opt.is_repeatable && usage.used_preferred_names.contains(opt.name.preferred()) {
            return false;
        }

        if opt
            .exclusives_on
            .iter()
            .any(|name| usage.used_aliases.contains(name))
        {
            return false;
        }

        !usage.used_options.iter().any(|used_opt| {
            used_opt
                .exclusives_on
                .iter()
                .any(|name| opt.name.contains(name))
        })
    }

    fn make_generator_context<'a>(
        arg: &'a Arg,
        parsed: &ParsedLine,
        command: &str,
        subcommands: &[&str],
        cwd: &str,
        completing_option_arg: bool,
    ) -> Option<ResolvedGeneratorContext<'a>> {
        if arg.generators.is_empty() {
            return None;
        }

        Some(ResolvedGeneratorContext {
            arg,
            context: GeneratorContext {
                command: command.to_string(),
                subcommands: subcommands
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                tokens: parsed
                    .tokens
                    .iter()
                    .map(|token| token.text.clone())
                    .collect(),
                partial: parsed.partial.clone(),
                completing_option_arg,
                cwd: cwd.to_string(),
            },
        })
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

    fn named_candidate(
        active_name: &str,
        metadata: CandidateMetadata<'_>,
        kind: CandidateKind,
    ) -> CompletionCandidate {
        CompletionCandidate {
            name: active_name.to_string(),
            insert_value: metadata.insert_value.clone(),
            display_name: (active_name == metadata.names.preferred())
                .then(|| metadata.display_name.clone())
                .flatten(),
            description: metadata.description.clone(),
            icon: metadata.icon.clone(),
            priority: metadata.priority.unwrap_or(DEFAULT_CANDIDATE_PRIORITY),
            kind,
        }
    }

    fn get_subcommands<'a>(node: SpecNode<'a>) -> &'a [Subcommand] {
        match node {
            SpecNode::Root(spec) => &spec.subcommands,
            SpecNode::Sub(subcommand) => &subcommand.subcommands,
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
            "displayName": "Git 🌿",
            "icon": "🌿",
            "subcommands": [
                {"name": "commit", "description": "Record changes",
                 "displayName": "Commit ✍️",
                 "icon": "✍️",
                 "insertValue": "commit --verbose",
                 "priority": 90,
                 "options": [
                    {"name": ["-m", "--message"], "description": "Commit message",
                     "displayName": "Message 💬",
                     "icon": "💬",
                     "insertValue": "--message",
                     "priority": 75,
                     "args": {"name": "message", "suggestions": [
                        {"name": "feat", "insertValue": "feat: {cursor}", "priority": 88}
                     ]}}
                 ]},
                {"name": "compare", "description": "Compare branches"},
                {"name": "clone", "description": "Clone a repository"},
                {"name": "checkout", "description": "Switch branches",
                 "args": {
                    "name": "branch",
                    "generators": {
                        "script": "printf 'main\\nrelease\\n'",
                        "splitOn": "\n",
                        "scriptTimeout": 500
                    }
                 },
                 "options": [
                    {"name": "-b", "description": "Create and checkout new branch"}
                 ]},
                {"name": "add", "description": "Add files",
                 "args": {"template": "filepaths", "isVariadic": true}}
            ],
            "options": [
                {"name": "--version", "description": "Print version"},
                {"name": "--help", "description": "Show help", "isPersistent": true},
                {"name": "--config", "description": "Config file", "isPersistent": true,
                 "args": {"name": "config", "suggestions": ["dev.toml", "prod.toml"]}},
                {"name": "--tag", "description": "Apply tag", "isPersistent": true, "isRepeatable": true},
                {"name": "--json", "description": "JSON output", "isPersistent": true,
                 "exclusivesOn": ["--yaml"]},
                {"name": "--yaml", "description": "YAML output", "isPersistent": true,
                 "exclusivesOn": ["--json"]},
                {"name": "--root-only", "description": "Root-only option"}
            ]
        }"#;
        store.load_embedded(&[("git.json", git_spec)]);
        store
    }

    #[test]
    fn test_complete_subcommands() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git com", ".").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"compare"));
    }

    #[test]
    fn test_complete_subcommands_with_matcher() {
        use crate::completion::matcher::FuzzyMatcher;

        let mut engine = CompletionEngine::new(test_store());
        let completion = engine.complete("git com", ".");
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
        let candidates = engine.complete("git commit -", ".").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"-m"));
        assert!(names.contains(&"--message"));
        assert!(names.contains(&"--help"));
        assert!(names.contains(&"--config"));
        assert!(!names.contains(&"--root-only"));
    }

    #[test]
    fn test_complete_options_filters_used_non_repeatable_flags() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git commit --help -", ".").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(!names.contains(&"--help"));
        assert!(names.contains(&"--config"));
    }

    #[test]
    fn test_complete_options_keeps_repeatable_flags_visible() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git commit --tag -", ".").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"--tag"));
    }

    #[test]
    fn test_complete_options_hides_mutually_exclusive_flags() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git commit --json -", ".").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(!names.contains(&"--yaml"));
    }

    #[test]
    fn test_complete_inherited_option_args() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git commit --config ", ".").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"dev.toml"));
        assert!(names.contains(&"prod.toml"));
    }

    #[test]
    fn test_no_completions_after_non_template_arg() {
        let mut engine = CompletionEngine::new(test_store());
        let candidates = engine.complete("git clone ", ".").candidates;
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
        let candidates = engine.complete("git ", ".").candidates;
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"clone"));
        assert!(names.contains(&"checkout"));
    }

    #[test]
    fn test_complete_candidates_preserve_display_metadata() {
        let mut engine = CompletionEngine::new(test_store());
        let commit = engine
            .complete("git com", ".")
            .candidates
            .into_iter()
            .find(|candidate| candidate.name == "commit")
            .unwrap();
        assert_eq!(commit.display_name.as_deref(), Some("Commit ✍️"));
        assert_eq!(commit.icon.as_deref(), Some("✍️"));
        assert_eq!(commit.insert_value.as_deref(), Some("commit --verbose"));
        assert_eq!(commit.priority, 90);

        let root = engine
            .complete("", ".")
            .candidates
            .into_iter()
            .find(|candidate| candidate.name == "git")
            .unwrap();
        assert_eq!(root.display_name.as_deref(), Some("Git 🌿"));
        assert_eq!(root.icon.as_deref(), Some("🌿"));
    }

    #[test]
    fn test_complete_option_arg_preserves_insert_value_and_priority() {
        let mut engine = CompletionEngine::new(test_store());
        let candidate = engine
            .complete("git commit -m ", ".")
            .candidates
            .into_iter()
            .find(|candidate| candidate.name == "feat")
            .unwrap();

        assert_eq!(candidate.insert_value.as_deref(), Some("feat: {cursor}"));
        assert_eq!(candidate.priority, 88);
    }

    #[test]
    fn test_complete_positional_runs_generators() {
        let mut engine = CompletionEngine::new(test_store());
        let names: Vec<String> = engine
            .complete("git checkout ", ".")
            .candidates
            .into_iter()
            .map(|candidate| candidate.name)
            .collect();

        assert!(names.contains(&"main".to_string()));
        assert!(names.contains(&"release".to_string()));
    }
}
