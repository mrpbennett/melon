use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum ShellType {
    Zsh,
    Bash,
    Fish,
    Unknown(String),
}

impl ShellType {
    pub fn name(&self) -> &str {
        match self {
            ShellType::Zsh => "zsh",
            ShellType::Bash => "bash",
            ShellType::Fish => "fish",
            ShellType::Unknown(s) => s,
        }
    }
}

/// Detect the user's shell from $SHELL, falling back to /bin/zsh.
pub fn detect_shell() -> (PathBuf, ShellType) {
    let shell_path = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let shell_type = classify_shell(&shell_path);
    (PathBuf::from(shell_path), shell_type)
}

fn classify_shell(path: &str) -> ShellType {
    if path.ends_with("/zsh") || path.ends_with("/zsh5") {
        ShellType::Zsh
    } else if path.ends_with("/bash") {
        ShellType::Bash
    } else if path.ends_with("/fish") {
        ShellType::Fish
    } else {
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        ShellType::Unknown(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_zsh() {
        assert_eq!(classify_shell("/bin/zsh"), ShellType::Zsh);
        assert_eq!(classify_shell("/usr/local/bin/zsh"), ShellType::Zsh);
    }

    #[test]
    fn test_classify_bash() {
        assert_eq!(classify_shell("/bin/bash"), ShellType::Bash);
    }

    #[test]
    fn test_classify_unknown() {
        assert!(matches!(classify_shell("/bin/tcsh"), ShellType::Unknown(_)));
    }
}
