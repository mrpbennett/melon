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
    println!();
    println!("if [ -z \"$MELON\" ] && [ -t 0 ] && [ -t 1 ]; then");
    println!("  exec {exe}");
    println!("fi");
}
