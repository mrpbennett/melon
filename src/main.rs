use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "melon", version, about = "Terminal autocomplete engine")]
struct Cli {
    /// Enable debug logging to ~/.local/share/melon/melon.log
    #[arg(long)]
    debug: bool,

    /// Print shell integration snippet (add to .zshrc/.bashrc)
    #[arg(long)]
    install: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.install {
        print_install_snippet();
        return Ok(());
    }

    // Set up logging
    if cli.debug {
        let log_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("melon");
        std::fs::create_dir_all(&log_dir)?;
        let log_file = std::fs::File::create(log_dir.join("melon.log"))?;
        tracing_subscriber::fmt()
            .with_writer(log_file)
            .with_env_filter("melon=debug")
            .init();
    }

    let exit_code = melon::pty::proxy::run_proxy().await?;
    std::process::exit(exit_code);
}

fn print_install_snippet() {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "melon".to_string());

    println!("# Add this to your shell rc file (~/.zshrc or ~/.bashrc):");
    println!("# It wraps your shell in melon for autocomplete support.");
    println!("# When running inside melon, emit OSC 7 so melon can track shell cwd.");
    println!();
    println!("if [ -n \"$MELON\" ]; then");
    println!("  __melon_osc7() {{");
    println!("    printf '\\033]7;file://%s%s\\007' \"${{HOSTNAME:-localhost}}\" \"$PWD\"");
    println!("  }}");
    println!("  if [ -n \"$ZSH_VERSION\" ]; then");
    println!("    autoload -Uz add-zsh-hook 2>/dev/null");
    println!(
        "    add-zsh-hook precmd __melon_osc7 2>/dev/null || precmd_functions+=(__melon_osc7)"
    );
    println!("  elif [ -n \"$BASH_VERSION\" ]; then");
    println!("    case \";${{PROMPT_COMMAND}};\" in");
    println!("      *\";__melon_osc7;\"*) ;;");
    println!("      *) PROMPT_COMMAND=\"__melon_osc7${{PROMPT_COMMAND:+;$PROMPT_COMMAND}}\" ;;");
    println!("    esac");
    println!("  fi");
    println!("fi");
    println!();
    println!("if [ -z \"$MELON\" ] && [ -t 0 ] && [ -t 1 ]; then");
    println!("  exec {exe}");
    println!("fi");
}
