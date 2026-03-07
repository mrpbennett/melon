use serde::Deserialize;

/// Top-level completion specification for a command.
/// Mirrors the Fig autocomplete Spec type.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    pub name: StringOrArray,
    #[serde(default)]
    pub description: Option<String>,
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
    pub icon: Option<String>,
    #[serde(default)]
    pub hidden: bool,
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
    Single(Arg),
    Multiple(Vec<Arg>),
}

impl ArgOrArgs {
    pub fn iter(&self) -> ArgOrArgsIter<'_> {
        match self {
            ArgOrArgs::None => ArgOrArgsIter::None,
            ArgOrArgs::Single(arg) => ArgOrArgsIter::Single(Some(arg)),
            ArgOrArgs::Multiple(args) => ArgOrArgsIter::Multiple(args.iter()),
        }
    }

    pub fn first(&self) -> Option<&Arg> {
        match self {
            ArgOrArgs::None => None,
            ArgOrArgs::Single(arg) => Some(arg),
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
    /// Optional display description.
    pub description: Option<String>,
    /// Type of completion for icon/styling.
    pub kind: CandidateKind,
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
            "subcommands": [
                {
                    "name": ["commit", "ci"],
                    "description": "Record changes to the repository",
                    "options": [
                        {
                            "name": ["-m", "--message"],
                            "description": "Commit message",
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
        assert_eq!(
            spec.subcommands[0].name.iter().collect::<Vec<_>>(),
            vec!["commit", "ci"]
        );
        assert_eq!(spec.subcommands[0].options.len(), 2);
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
                    {"name": "maybe", "description": "Perhaps"}
                ]
            }
        }"#;

        let spec: Spec = serde_json::from_str(json).unwrap();
        let args = spec.args.as_slice();
        assert_eq!(args[0].suggestions.len(), 3);
    }
}
