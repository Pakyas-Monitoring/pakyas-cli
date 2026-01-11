use crate::cli::Cli;
use anyhow::Result;
use clap::CommandFactory;
use clap_complete::{Shell, generate};
use std::io;

/// Generate shell completions
///
/// Outputs completion script to stdout. Users can redirect to appropriate file:
///   pakyas completion bash > ~/.local/share/bash-completion/completions/pakyas
///   pakyas completion zsh > ~/.zfunc/_pakyas
///   pakyas completion fish > ~/.config/fish/completions/pakyas.fish
pub fn generate_completions(shell: Shell) -> Result<()> {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();

    generate(shell, &mut cmd, name, &mut io::stdout());

    // Print helpful message to stderr (doesn't interfere with redirected stdout)
    match shell {
        Shell::Bash => {
            eprintln!();
            eprintln!("# Add completions to bash:");
            eprintln!(
                "# pakyas completion bash > ~/.local/share/bash-completion/completions/pakyas"
            );
            eprintln!("# Or: pakyas completion bash >> ~/.bashrc");
        }
        Shell::Zsh => {
            eprintln!();
            eprintln!("# Add completions to zsh:");
            eprintln!("# pakyas completion zsh > ~/.zfunc/_pakyas");
            eprintln!(
                "# Then add to ~/.zshrc: fpath=(~/.zfunc $fpath); autoload -Uz compinit && compinit"
            );
        }
        Shell::Fish => {
            eprintln!();
            eprintln!("# Add completions to fish:");
            eprintln!("# pakyas completion fish > ~/.config/fish/completions/pakyas.fish");
        }
        _ => {}
    }

    Ok(())
}
