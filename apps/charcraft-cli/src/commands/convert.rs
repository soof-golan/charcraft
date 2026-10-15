//! `convert`: read a puppet and write it again.

use color_eyre::eyre::{Result, WrapErr, bail};

use crate::cli::ConvertArgs;
use crate::source::{Kind, detect, open_project};

/// Runs the command.
pub fn run(args: &ConvertArgs) -> Result<()> {
    if detect(&args.input)? != Kind::Puppet {
        bail!("{} is not a .puppet file", args.input.display());
    }
    let mut project = open_project(&args.input)?;
    project.save_puppet(&args.output).wrap_err_with(|| format!("writing {}", args.output.display()))?;
    println!("{} -> {}", args.input.display(), args.output.display());
    Ok(())
}
