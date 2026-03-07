use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::source::{CompletionContext, CompletionSource, PathSource};
use super::spec::{
    Arg, CandidateKind, CompletionCandidate, Generator, GeneratorCacheStrategy, GeneratorScript,
    Suggestion, SuggestionOrString, Template, TemplateKind, DEFAULT_CANDIDATE_PRIORITY,
};

#[derive(Debug, Clone)]
pub struct GeneratorContext {
    pub command: String,
    pub subcommands: Vec<String>,
    pub tokens: Vec<String>,
    pub partial: String,
    pub completing_option_arg: bool,
    pub cwd: String,
}

#[derive(Default)]
pub struct GeneratorSource {
    path_source: PathSource,
    session_cache: HashMap<SessionCacheKey, SessionCacheEntry>,
    shared_cache: HashMap<SharedCacheKey, SharedCacheEntry>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SessionCacheKey {
    generator_signature: String,
    command: String,
    subcommands: Vec<String>,
    tokens: Vec<String>,
    cwd: String,
    completing_option_arg: bool,
}

#[derive(Clone)]
struct SessionCacheEntry {
    trigger_key: Option<String>,
    candidates: Vec<CompletionCandidate>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct SharedCacheKey {
    key: String,
    cwd: Option<String>,
}

#[derive(Clone)]
struct SharedCacheEntry {
    created_at: Instant,
    candidates: Vec<CompletionCandidate>,
}

impl GeneratorSource {
    pub fn candidates(
        &mut self,
        arg: &Arg,
        context: &GeneratorContext,
    ) -> Vec<CompletionCandidate> {
        let mut candidates = Vec::new();
        for generator in arg.generators.iter() {
            candidates.extend(self.candidates_for_generator(generator, context));
        }
        candidates
    }

    fn candidates_for_generator(
        &mut self,
        generator: &Generator,
        context: &GeneratorContext,
    ) -> Vec<CompletionCandidate> {
        let session_key = SessionCacheKey {
            generator_signature: generator_signature(generator),
            command: context.command.clone(),
            subcommands: context.subcommands.clone(),
            tokens: context.tokens.clone(),
            cwd: context.cwd.clone(),
            completing_option_arg: context.completing_option_arg,
        };
        let trigger_key = trigger_key(generator, &context.partial);

        if let Some(entry) = self.session_cache.get(&session_key) {
            if entry.trigger_key == trigger_key {
                return entry.candidates.clone();
            }
        }

        let candidates = self.resolve_candidates(generator, context);
        self.session_cache.insert(
            session_key,
            SessionCacheEntry {
                trigger_key,
                candidates: candidates.clone(),
            },
        );
        candidates
    }

    fn resolve_candidates(
        &mut self,
        generator: &Generator,
        context: &GeneratorContext,
    ) -> Vec<CompletionCandidate> {
        let mut stale = None;
        if let Some(key) = shared_cache_key(generator, context) {
            if let Some(entry) = self.shared_cache.get(&key) {
                if cache_is_fresh(generator, entry.created_at) {
                    return entry.candidates.clone();
                }
                stale = Some(entry.candidates.clone());
            }

            let generated = self.generate_uncached(generator, context);
            if generated.is_empty()
                && matches!(
                    generator
                        .cache
                        .as_ref()
                        .and_then(|cache| cache.strategy.as_ref()),
                    Some(GeneratorCacheStrategy::StaleWhileRevalidate)
                )
            {
                return stale.unwrap_or_default();
            }

            self.shared_cache.insert(
                key,
                SharedCacheEntry {
                    created_at: Instant::now(),
                    candidates: generated.clone(),
                },
            );
            return generated;
        }

        self.generate_uncached(generator, context)
    }

    fn generate_uncached(
        &mut self,
        generator: &Generator,
        context: &GeneratorContext,
    ) -> Vec<CompletionCandidate> {
        if let Some(template) = &generator.template {
            return self.template_candidates(template, context);
        }

        let Some(script) = &generator.script else {
            return vec![];
        };

        let output = match execute_script(script, generator.script_timeout, context) {
            Some(output) => output,
            None => return vec![],
        };

        parse_script_output(&output)
            .or_else(|| split_output(generator, &output))
            .unwrap_or_default()
    }

    fn template_candidates(
        &mut self,
        template: &Template,
        context: &GeneratorContext,
    ) -> Vec<CompletionCandidate> {
        if !uses_path_template(template) {
            return vec![];
        }

        let path_context = CompletionContext {
            command: context.command.clone(),
            subcommands: context.subcommands.clone(),
            partial: context.partial.clone(),
            completing_option_arg: context.completing_option_arg,
            cwd: context.cwd.clone(),
        };
        self.path_source.candidates(&path_context)
    }
}

fn execute_script(
    script: &GeneratorScript,
    timeout_ms: Option<u64>,
    context: &GeneratorContext,
) -> Option<String> {
    let mut command = command_for_script(script)?;
    command.current_dir(&context.cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::null());
    command.env("MELON_COMMAND", &context.command);
    command.env(
        "MELON_SUBCOMMANDS_JSON",
        serde_json::to_string(&context.subcommands).ok()?,
    );
    command.env(
        "MELON_TOKENS_JSON",
        serde_json::to_string(&context.tokens).ok()?,
    );
    command.env("MELON_PARTIAL", &context.partial);
    command.env("MELON_CWD", &context.cwd);
    command.env(
        "MELON_COMPLETING_OPTION_ARG",
        if context.completing_option_arg {
            "1"
        } else {
            "0"
        },
    );

    let timeout = Duration::from_millis(timeout_ms.unwrap_or(250));
    let start = Instant::now();
    let mut child = command.spawn().ok()?;

    loop {
        match child.try_wait().ok()? {
            Some(_) => break,
            None if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => thread::sleep(Duration::from_millis(5)),
        }
    }

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8(output.stdout).ok()
}

fn command_for_script(script: &GeneratorScript) -> Option<Command> {
    match script {
        GeneratorScript::Single(value) => {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            let mut command = Command::new(shell);
            command.arg("-lc").arg(value);
            Some(command)
        }
        GeneratorScript::Multiple(parts) => {
            let (program, args) = parts.split_first()?;
            let mut command = Command::new(program);
            command.args(args);
            Some(command)
        }
    }
}

fn parse_script_output(output: &str) -> Option<Vec<CompletionCandidate>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Some(vec![]);
    }

    let parsed = serde_json::from_str::<Vec<SuggestionOrString>>(trimmed).ok()?;
    Some(
        parsed
            .iter()
            .flat_map(|suggestion| suggestion_candidates(suggestion, CandidateKind::Argument))
            .collect(),
    )
}

fn split_output(generator: &Generator, output: &str) -> Option<Vec<CompletionCandidate>> {
    let delimiter = generator.split_on.as_deref()?;
    let mut candidates = Vec::new();
    for value in output.split(delimiter) {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        candidates.push(CompletionCandidate {
            name: value.to_string(),
            insert_value: None,
            display_name: None,
            description: None,
            icon: None,
            priority: DEFAULT_CANDIDATE_PRIORITY,
            kind: CandidateKind::Argument,
        });
    }
    Some(candidates)
}

pub(crate) fn suggestion_candidates(
    suggestion: &SuggestionOrString,
    kind: CandidateKind,
) -> Vec<CompletionCandidate> {
    match suggestion {
        SuggestionOrString::String(value) => vec![CompletionCandidate {
            name: value.clone(),
            insert_value: None,
            display_name: None,
            description: None,
            icon: None,
            priority: DEFAULT_CANDIDATE_PRIORITY,
            kind,
        }],
        SuggestionOrString::Suggestion(value) => structured_suggestion_candidates(value, kind),
    }
}

fn structured_suggestion_candidates(
    suggestion: &Suggestion,
    kind: CandidateKind,
) -> Vec<CompletionCandidate> {
    if suggestion.hidden {
        return vec![];
    }

    let mut candidates = Vec::new();
    for name in suggestion.name.iter() {
        candidates.push(CompletionCandidate {
            name: name.to_string(),
            insert_value: suggestion.insert_value.clone(),
            display_name: (name == suggestion.name.primary())
                .then(|| suggestion.display_name.clone())
                .flatten(),
            description: suggestion.description.clone(),
            icon: suggestion.icon.clone(),
            priority: suggestion.priority.unwrap_or(DEFAULT_CANDIDATE_PRIORITY),
            kind: kind.clone(),
        });
    }
    candidates
}

fn uses_path_template(template: &Template) -> bool {
    match template {
        Template::Single(kind) => matches!(kind, TemplateKind::Filepaths | TemplateKind::Folders),
        Template::Multiple(kinds) => kinds
            .iter()
            .any(|kind| matches!(kind, TemplateKind::Filepaths | TemplateKind::Folders)),
    }
}

fn generator_signature(generator: &Generator) -> String {
    let template = match &generator.template {
        Some(Template::Single(kind)) => format!("single:{kind:?}"),
        Some(Template::Multiple(kinds)) => format!("multiple:{kinds:?}"),
        None => "none".to_string(),
    };
    let script = match &generator.script {
        Some(GeneratorScript::Single(value)) => format!("single:{value}"),
        Some(GeneratorScript::Multiple(values)) => format!("multiple:{values:?}"),
        None => "none".to_string(),
    };
    let trigger = generator
        .trigger
        .as_ref()
        .map(|value| value.as_str())
        .unwrap_or_default();
    let cache = generator
        .cache
        .as_ref()
        .map(|cache| {
            format!(
                "{:?}:{}:{}:{}",
                cache.strategy,
                cache.ttl.unwrap_or(0),
                cache.cache_by_directory,
                cache.cache_key.as_deref().unwrap_or_default()
            )
        })
        .unwrap_or_default();

    format!(
        "{template}|{script}|{}|{trigger}|{cache}",
        generator.split_on.as_deref().unwrap_or_default()
    )
}

fn trigger_key(generator: &Generator, partial: &str) -> Option<String> {
    let trigger = generator.trigger.as_ref()?.as_str();
    let index = partial.rfind(trigger)?;
    Some(partial[..index + trigger.len()].to_string())
}

fn shared_cache_key(generator: &Generator, context: &GeneratorContext) -> Option<SharedCacheKey> {
    let cache = generator.cache.as_ref()?;
    Some(SharedCacheKey {
        key: cache
            .cache_key
            .clone()
            .unwrap_or_else(|| generator_signature(generator)),
        cwd: cache.cache_by_directory.then(|| context.cwd.clone()),
    })
}

fn cache_is_fresh(generator: &Generator, created_at: Instant) -> bool {
    let Some(cache) = generator.cache.as_ref() else {
        return false;
    };
    let Some(ttl_ms) = cache.ttl else {
        return true;
    };
    created_at.elapsed() <= Duration::from_millis(ttl_ms)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::completion::spec::{
        GeneratorCache, GeneratorCacheStrategy, GeneratorOrGenerators, GeneratorTrigger,
    };

    fn context(cwd: &str, partial: &str) -> GeneratorContext {
        GeneratorContext {
            command: "git".into(),
            subcommands: vec!["checkout".into()],
            tokens: vec!["git".into(), "checkout".into()],
            partial: partial.into(),
            completing_option_arg: false,
            cwd: cwd.into(),
        }
    }

    #[test]
    fn test_split_script_output_to_candidates() {
        let mut source = GeneratorSource::default();
        let arg = Arg {
            name: Some("branch".into()),
            description: None,
            suggestions: vec![],
            generators: GeneratorOrGenerators::Single(Generator {
                template: None,
                script: Some(GeneratorScript::Single("printf 'main\\nrelease\\n'".into())),
                script_timeout: Some(500),
                split_on: Some("\n".into()),
                trigger: None,
                cache: None,
            }),
            template: None,
            is_optional: false,
            is_variadic: false,
            default: None,
        };

        let candidates = source.candidates(&arg, &context(".", ""));
        let names: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect();
        assert_eq!(names, vec!["main", "release"]);
    }

    #[test]
    fn test_json_script_output_preserves_metadata() {
        let mut source = GeneratorSource::default();
        let arg = Arg {
            name: Some("env".into()),
            description: None,
            suggestions: vec![],
            generators: GeneratorOrGenerators::Single(Generator {
                template: None,
                script: Some(GeneratorScript::Single(
                    "printf '[{\"name\":\"prod\",\"icon\":\"🚀\",\"insertValue\":\"deploy --env=prod\",\"priority\":95}]'"
                        .into(),
                )),
                script_timeout: Some(500),
                split_on: None,
                trigger: None,
                cache: None,
            }),
            template: None,
            is_optional: false,
            is_variadic: false,
            default: None,
        };

        let candidates = source.candidates(&arg, &context(".", ""));
        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.name, "prod");
        assert_eq!(candidate.insert_value.as_deref(), Some("deploy --env=prod"));
        assert_eq!(candidate.icon.as_deref(), Some("🚀"));
        assert_eq!(candidate.priority, 95);
    }

    #[test]
    fn test_session_cache_reuses_results_without_trigger_change() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("branches.txt");
        fs::write(&output_path, "main\n").unwrap();

        let script = format!("cat {}", output_path.display());
        let mut source = GeneratorSource::default();
        let arg = Arg {
            name: Some("branch".into()),
            description: None,
            suggestions: vec![],
            generators: GeneratorOrGenerators::Single(Generator {
                template: None,
                script: Some(GeneratorScript::Single(script)),
                script_timeout: Some(500),
                split_on: Some("\n".into()),
                trigger: None,
                cache: None,
            }),
            template: None,
            is_optional: false,
            is_variadic: false,
            default: None,
        };

        let first = source.candidates(&arg, &context(dir.path().to_str().unwrap(), "m"));
        fs::write(&output_path, "release\n").unwrap();
        let second = source.candidates(&arg, &context(dir.path().to_str().unwrap(), "ma"));

        assert_eq!(first[0].name, "main");
        assert_eq!(second[0].name, "main");
    }

    #[test]
    fn test_trigger_invalidates_session_cache() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("dirs.txt");
        fs::write(&output_path, "src/main.rs\n").unwrap();

        let script = format!("cat {}", output_path.display());
        let mut source = GeneratorSource::default();
        let arg = Arg {
            name: Some("path".into()),
            description: None,
            suggestions: vec![],
            generators: GeneratorOrGenerators::Single(Generator {
                template: None,
                script: Some(GeneratorScript::Single(script)),
                script_timeout: Some(500),
                split_on: Some("\n".into()),
                trigger: Some(GeneratorTrigger::Literal("/".into())),
                cache: None,
            }),
            template: None,
            is_optional: false,
            is_variadic: false,
            default: None,
        };

        let first = source.candidates(&arg, &context(dir.path().to_str().unwrap(), "src"));
        fs::write(&output_path, "src/bin.rs\n").unwrap();
        let second = source.candidates(&arg, &context(dir.path().to_str().unwrap(), "src/"));

        assert_eq!(first[0].name, "src/main.rs");
        assert_eq!(second[0].name, "src/bin.rs");
    }

    #[test]
    fn test_explicit_cache_reuses_results_across_contexts() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("branches.txt");
        fs::write(&output_path, "main\n").unwrap();

        let script = format!("cat {}", output_path.display());
        let mut source = GeneratorSource::default();
        let arg = Arg {
            name: Some("branch".into()),
            description: None,
            suggestions: vec![],
            generators: GeneratorOrGenerators::Single(Generator {
                template: None,
                script: Some(GeneratorScript::Single(script)),
                script_timeout: Some(500),
                split_on: Some("\n".into()),
                trigger: None,
                cache: Some(GeneratorCache {
                    strategy: Some(GeneratorCacheStrategy::MaxAge),
                    ttl: Some(60_000),
                    cache_by_directory: true,
                    cache_key: Some("branches".into()),
                }),
            }),
            template: None,
            is_optional: false,
            is_variadic: false,
            default: None,
        };

        let first = source.candidates(&arg, &context(dir.path().to_str().unwrap(), ""));
        fs::write(&output_path, "release\n").unwrap();
        let mut other_context = context(dir.path().to_str().unwrap(), "");
        other_context.tokens.push("--all".into());
        let second = source.candidates(&arg, &other_context);

        assert_eq!(first[0].name, "main");
        assert_eq!(second[0].name, "main");
    }
}
