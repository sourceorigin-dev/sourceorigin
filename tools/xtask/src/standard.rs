//! `cargo xtask standard`: the engineering standard's rules, from the
//! `sourceorigin-standard` crate, whose README is the written standard.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use sourceorigin_standard::rules;
use std::{path::Path, process::ExitCode};

/// Checks the repository against the standard, or another of its tasks.
#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct Standard {
    /// First make the fixes that need no judgement, such as a budget that is
    /// higher than its count, then check.
    #[arg(long)]
    fix: bool,
    /// Another task than checking.
    #[command(subcommand)]
    task: Option<Task>,
}

/// The standard's tasks besides checking.
#[derive(Subcommand)]
enum Task {
    /// Print each rule's violations and deviation, the `#[expect]` attributes
    /// and the allowed lints, as JSON for the cleanup pass.
    Report,
    /// List the rules.
    Rules,
    /// Write the standard's shared skills into `.agents/skills`.
    Skills,
}

impl Standard {
    /// Runs the task on the checkout at `root`, failing the process when the
    /// check finds violations.
    pub(crate) fn run(self, root: &Path) -> Result<ExitCode> {
        match self.task {
            None => {
                if self.fix {
                    for fix in rules::fix(root).context("fix the repository")? {
                        println!("fixed {fix}");
                    }
                }
                let violations =
                    rules::check(root).context("check the repository against the standard")?;
                for violation in &violations {
                    eprintln!("{violation}");
                }
                if violations.is_empty() {
                    return Ok(ExitCode::SUCCESS);
                }
                eprintln!(
                    "{} violations of the standard; `cargo xtask standard rules` lists the rules",
                    violations.len()
                );
                Ok(ExitCode::FAILURE)
            }
            Some(Task::Report) => {
                let report = rules::report(root).context("report on the repository")?;
                println!("{}", report.to_json().context("print the report as JSON")?);
                Ok(ExitCode::SUCCESS)
            }
            Some(Task::Rules) => {
                print!("{}", rules::describe());
                Ok(ExitCode::SUCCESS)
            }
            Some(Task::Skills) => {
                for fix in rules::sync_skills(root).context("sync the skills")? {
                    println!("synced {fix}");
                }
                Ok(ExitCode::SUCCESS)
            }
        }
    }
}
