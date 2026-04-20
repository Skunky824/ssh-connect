mod app;
mod browser;
mod editor;
mod ssh;
mod ui;
mod vterm;

use anyhow::{Context, Result};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "ssh-connect",
    about = "SSH terminal with file browser (strict privacy mode ON by default)"
)]
struct Args {
    /// Target in user@host format
    target: String,

    /// SSH port
    #[arg(short, long, default_value_t = 22)]
    port: u16,

    /// Path to identity file (private key)
    #[arg(short, long)]
    identity: Option<String>,

    /// Strict privacy mode (default ON): never send helper shell commands.
    #[arg(long, default_value_t = true)]
    strict_privacy: bool,

    /// Disable strict privacy mode (allows helper shell fallbacks when needed).
    #[arg(long, conflicts_with = "strict_privacy")]
    no_strict_privacy: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let strict_privacy = if args.no_strict_privacy {
        false
    } else {
        args.strict_privacy
    };

    let (user, host) = parse_target(&args.target)
        .context("Target must be in user@host format")?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(app::run(
        user,
        host,
        args.port,
        args.identity,
        strict_privacy,
    ))
}

fn parse_target(target: &str) -> Option<(String, String)> {
    let mut parts = target.splitn(2, '@');
    let user = parts.next()?.to_string();
    let host = parts.next()?.to_string();
    if user.is_empty() || host.is_empty() {
        return None;
    }
    Some((user, host))
}
