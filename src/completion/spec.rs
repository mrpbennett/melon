use serde::Deserialize;

pub const DEFAULT_CANDIDATE_PRIORITY: i32 = 50;

/// Top-level completion specification for a command.
/// Mirrors the Fig autocomplete Spec type.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub name: StringOrArray,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub insert_value: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub subcommands: Vec<Subcommand>,
    #[serde(default)]
    pub options: Vec<Opt>,
    #[serde(default)]
    pub args: ArgOrArgs,
}

/// A subcommand within a spec (e.g., `git commit`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subcommand {
    pub name: StringOrArray,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub insert_value: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub subcommands: Vec<Subcommand>,
    #[serde(default)]
    pub options: Vec<Opt>,
    #[serde(default)]
    pub args: ArgOrArgs,
    #[serde(default)]
    pub hidden: bool,
}

/// A command-line option/flag (e.g., `--verbose`, `-v`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Opt {
    pub name: StringOrArray,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub insert_value: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub args: ArgOrArgs,
    /// If true, this is a persistent/global option inherited by subcommands.
    #[serde(default)]
    pub is_persistent: bool,
    /// If true, the option is required.
    #[serde(default)]
    pub is_required: bool,
    /// If true, the option can be specified multiple times.
    #[serde(default)]
    pub is_repeatable: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Mutually exclusive options.
    #[serde(default)]
    pub exclusives_on: Vec<String>,
}

/// An argument to a command or option (e.g., `<file>`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arg {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Static suggestions for this argument.
    #[serde(default)]
    pub suggestions: Vec<SuggestionOrString>,
    /// Dynamic suggestion generators for this argument.
    #[serde(default)]
    pub generators: GeneratorOrGenerators,
    /// Template for special argument types.
    #[serde(default)]
    pub template: Option<Template>,
    /// If true, this argument is optional.
    #[serde(default)]
    pub is_optional: bool,
    /// If true, this argument is variadic (accepts multiple values).
    #[serde(default)]
    pub is_variadic: bool,
    /// Default value.
    #[serde(default)]
    pub default: Option<String>,
}

/// A suggestion can be a plain string or a structured object.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SuggestionOrString {
    String(String),
    Suggestion(Suggestion),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub name: StringOrArray,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub insert_value: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub hidden: bool,
}

/// Generators can be a single object, an array, or absent.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(untagged)]
pub enum GeneratorOrGenerators {
    #[default]
    None,
    Single(Generator),
    Multiple(Vec<Generator>),
}

impl GeneratorOrGenerators {
    pub fn iter(&self) -> GeneratorOrGeneratorsIter<'_> {
        match self {
            GeneratorOrGenerators::None => GeneratorOrGeneratorsIter::None,
            GeneratorOrGenerators::Single(generator) => {
                GeneratorOrGeneratorsIter::Single(Some(generator))
            }
            GeneratorOrGenerators::Multiple(generators) => {
                GeneratorOrGeneratorsIter::Multiple(generators.iter())
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, GeneratorOrGenerators::None)
    }
}

pub enum GeneratorOrGeneratorsIter<'a> {
    None,
    Single(Option<&'a Generator>),
    Multiple(std::slice::Iter<'a, Generator>),
}

impl<'a> Iterator for GeneratorOrGeneratorsIter<'a> {
    type Item = &'a Generator;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            GeneratorOrGeneratorsIter::None => None,
            GeneratorOrGeneratorsIter::Single(value) => value.take(),
            GeneratorOrGeneratorsIter::Multiple(iter) => iter.next(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    #[serde(default)]
    pub template: Option<Template>,
    #[serde(default)]
    pub script: Option<GeneratorScript>,
    #[serde(default)]
    pub script_timeout: Option<u64>,
    #[serde(default)]
    pub split_on: Option<String>,
    #[serde(default)]
    pub trigger: Option<GeneratorTrigger>,
    #[serde(default)]
    pub cache: Option<GeneratorCache>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GeneratorScript {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum GeneratorTrigger {
    Literal(String),
}

impl GeneratorTrigger {
    pub fn as_str(&self) -> &str {
        match self {
            GeneratorTrigger::Literal(value) => value,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorCache {
    #[serde(default)]
    pub strategy: Option<GeneratorCacheStrategy>,
    #[serde(default)]
    pub ttl: Option<u64>,
    #[serde(default)]
    pub cache_by_directory: bool,
    #[serde(default)]
    pub cache_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GeneratorCacheStrategy {
    MaxAge,
    StaleWhileRevalidate,
}

/// Template types for special completion sources.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Template {
    Single(TemplateKind),
    Multiple(Vec<TemplateKind>),
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TemplateKind {
    Filepaths,
    Folders,
    History,
    Help,
}

/// A name field can be a single string or an array of aliases.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrArray {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrArray {
    /// Get the primary (first) name.
    pub fn primary(&self) -> &str {
        match self {
            StringOrArray::Single(s) => s,
            StringOrArray::Multiple(v) => v.first().map(|s| s.as_str()).unwrap_or(""),
        }
    }

    /// Iterate over all name variants without allocating.
    pub fn iter(&self) -> StringOrArrayIter<'_> {
        match self {
            StringOrArray::Single(s) => StringOrArrayIter::Single(Some(s.as_str())),
            StringOrArray::Multiple(v) => StringOrArrayIter::Multiple(v.iter()),
        }
    }

    /// Check whether any alias exactly matches `needle`.
    pub fn contains(&self, needle: &str) -> bool {
        self.iter().any(|name| name == needle)
    }

    /// Pick a stable display name for UI output, preferring the long option form.
    pub fn preferred(&self) -> &str {
        self.iter()
            .find(|name| name.starts_with("--"))
            .unwrap_or_else(|| self.primary())
    }
}

pub enum StringOrArrayIter<'a> {
    Single(Option<&'a str>),
    Multiple(std::slice::Iter<'a, String>),
}

impl<'a> Iterator for StringOrArrayIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            StringOrArrayIter::Single(value) => value.take(),
            StringOrArrayIter::Multiple(iter) => iter.next().map(|value| value.as_str()),
        }
    }
}

/// Args can be a single arg, an array of args, or absent.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(untagged)]
pub enum ArgOrArgs {
    #[default]
    None,
    Single(Box<Arg>),
    Multiple(Vec<Arg>),
}

impl ArgOrArgs {
    pub fn iter(&self) -> ArgOrArgsIter<'_> {
        match self {
            ArgOrArgs::None => ArgOrArgsIter::None,
            ArgOrArgs::Single(arg) => ArgOrArgsIter::Single(Some(arg.as_ref())),
            ArgOrArgs::Multiple(args) => ArgOrArgsIter::Multiple(args.iter()),
        }
    }

    pub fn first(&self) -> Option<&Arg> {
        match self {
            ArgOrArgs::None => None,
            ArgOrArgs::Single(arg) => Some(arg.as_ref()),
            ArgOrArgs::Multiple(args) => args.first(),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, ArgOrArgs::None)
    }

    pub fn as_slice(&self) -> Vec<&Arg> {
        self.iter().collect()
    }
}

pub enum ArgOrArgsIter<'a> {
    None,
    Single(Option<&'a Arg>),
    Multiple(std::slice::Iter<'a, Arg>),
}

impl<'a> Iterator for ArgOrArgsIter<'a> {
    type Item = &'a Arg;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ArgOrArgsIter::None => None,
            ArgOrArgsIter::Single(value) => value.take(),
            ArgOrArgsIter::Multiple(iter) => iter.next(),
        }
    }
}

/// A candidate produced by the completion engine.
#[derive(Debug, Clone)]
pub struct CompletionCandidate {
    /// The text to insert.
    pub name: String,
    /// Optional inserted text when it differs from the match/filter name.
    pub insert_value: Option<String>,
    /// Optional display label while keeping `name` as the inserted value.
    pub display_name: Option<String>,
    /// Optional display description.
    pub description: Option<String>,
    /// Optional icon metadata from the spec.
    pub icon: Option<String>,
    /// Rank hint from the completion spec.
    pub priority: i32,
    /// Type of completion for icon/styling.
    pub kind: CandidateKind,
}

impl CompletionCandidate {
    pub fn display_label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    pub fn insert_text(&self) -> &str {
        self.insert_value.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CandidateKind {
    Subcommand,
    Option,
    Argument,
    File,
    Folder,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_simple_spec() {
        let json = r#"{
            "name": "git",
            "description": "The version control system",
            "insertValue": "git",
            "displayName": "Git 🌿",
            "icon": "🌿",
            "priority": 77,
            "subcommands": [
                {
                    "name": ["commit", "ci"],
                    "description": "Record changes to the repository",
                    "insertValue": "commit --verbose",
                    "displayName": "Commit ✍️",
                    "icon": "✍️",
                    "priority": 82,
                    "options": [
                        {
                            "name": ["-m", "--message"],
                            "description": "Commit message",
                            "insertValue": "--message",
                            "displayName": "Message 💬",
                            "icon": "💬",
                            "priority": 66,
                            "args": {
                                "name": "message"
                            }
                        },
                        {
                            "name": "--amend",
                            "description": "Amend the previous commit"
                        }
                    ]
                }
            ],
            "options": [
                {
                    "name": "--version",
                    "description": "Print version"
                }
            ]
        }"#;

        let spec: Spec = serde_json::from_str(json).unwrap();
        assert_eq!(spec.name.primary(), "git");
        assert_eq!(spec.subcommands.len(), 1);
        assert_eq!(spec.subcommands[0].name.primary(), "commit");
        assert_eq!(spec.insert_value.as_deref(), Some("git"));
        assert_eq!(spec.priority, Some(77));
        assert_eq!(spec.display_name.as_deref(), Some("Git 🌿"));
        assert_eq!(spec.icon.as_deref(), Some("🌿"));
        assert_eq!(
            spec.subcommands[0].insert_value.as_deref(),
            Some("commit --verbose")
        );
        assert_eq!(spec.subcommands[0].priority, Some(82));
        assert_eq!(
            spec.subcommands[0].display_name.as_deref(),
            Some("Commit ✍️")
        );
        assert_eq!(spec.subcommands[0].icon.as_deref(), Some("✍️"));
        assert_eq!(
            spec.subcommands[0].name.iter().collect::<Vec<_>>(),
            vec!["commit", "ci"]
        );
        assert_eq!(spec.subcommands[0].options.len(), 2);
        assert_eq!(
            spec.subcommands[0].options[0].display_name.as_deref(),
            Some("Message 💬")
        );
        assert_eq!(
            spec.subcommands[0].options[0].insert_value.as_deref(),
            Some("--message")
        );
        assert_eq!(spec.subcommands[0].options[0].priority, Some(66));
        assert_eq!(spec.subcommands[0].options[0].icon.as_deref(), Some("💬"));
        assert_eq!(spec.options.len(), 1);
    }

    #[test]
    fn test_deserialize_arg_template() {
        let json = r#"{
            "name": "cat",
            "args": {
                "template": "filepaths",
                "isVariadic": true
            }
        }"#;

        let spec: Spec = serde_json::from_str(json).unwrap();
        let args = spec.args.as_slice();
        assert_eq!(args.len(), 1);
        assert!(args[0].is_variadic);
        assert!(matches!(
            args[0].template,
            Some(Template::Single(TemplateKind::Filepaths))
        ));
    }

    #[test]
    fn test_deserialize_suggestions() {
        let json = r#"{
            "name": "test",
            "args": {
                "suggestions": [
                    "yes",
                    "no",
                    {
                        "name": "maybe",
                        "description": "Perhaps",
                        "insertValue": "maybe?",
                        "displayName": "Maybe 🤔",
                        "icon": "🤔",
                        "priority": 61
                    }
                ]
            }
        }"#;

        let spec: Spec = serde_json::from_str(json).unwrap();
        let args = spec.args.as_slice();
        assert_eq!(args[0].suggestions.len(), 3);
        let SuggestionOrString::Suggestion(suggestion) = &args[0].suggestions[2] else {
            panic!("expected structured suggestion");
        };
        assert_eq!(suggestion.display_name.as_deref(), Some("Maybe 🤔"));
        assert_eq!(suggestion.icon.as_deref(), Some("🤔"));
        assert_eq!(suggestion.insert_value.as_deref(), Some("maybe?"));
        assert_eq!(suggestion.priority, Some(61));
    }

    #[test]
    fn test_deserialize_generators() {
        let json = r#"{
            "name": "git",
            "subcommands": [
                {
                    "name": "checkout",
                    "args": {
                        "name": "branch",
                        "generators": [
                            {
                                "script": "git branch --format='%(refname:short)'",
                                "splitOn": "\n",
                                "scriptTimeout": 250,
                                "trigger": "/",
                                "cache": {
                                    "strategy": "max-age",
                                    "ttl": 5000,
                                    "cacheByDirectory": true,
                                    "cacheKey": "git-branches"
                                }
                            },
                            {
                                "template": "folders"
                            }
                        ]
                    }
                }
            ]
        }"#;

        let spec: Spec = serde_json::from_str(json).unwrap();
        let args = spec.subcommands[0].args.as_slice();
        let generators: Vec<&Generator> = args[0].generators.iter().collect();
        assert_eq!(generators.len(), 2);
        assert!(matches!(
            generators[0].script,
            Some(GeneratorScript::Single(ref script))
                if script == "git branch --format='%(refname:short)'"
        ));
        assert_eq!(generators[0].split_on.as_deref(), Some("\n"));
        assert_eq!(generators[0].script_timeout, Some(250));
        assert_eq!(
            generators[0].trigger.as_ref().map(GeneratorTrigger::as_str),
            Some("/")
        );
        let cache = generators[0].cache.as_ref().unwrap();
        assert_eq!(cache.strategy, Some(GeneratorCacheStrategy::MaxAge));
        assert_eq!(cache.ttl, Some(5000));
        assert!(cache.cache_by_directory);
        assert_eq!(cache.cache_key.as_deref(), Some("git-branches"));
        assert!(matches!(
            generators[1].template,
            Some(Template::Single(TemplateKind::Folders))
        ));
    }
}
