//! `dvs-migrate`: moves a dvs project on local storage onto dvs-server, keeping who added
//! each version and when. Run by an admin with a migrate key from the project's Migration
//! page. It only reads the repo and local storage.

mod client;
mod commands;
mod git;
mod orphans;
mod report;
mod storage;
mod walk;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::time::{Instant, SystemTime};

use anyhow::Result;
use clap::{Parser, Subcommand};
use dvs::ErrorCode;
use dvs::backends::server::coded;
use dvs::operation::{duration_ms, rfc3339_ms};
use url::Url;

use crate::client::{Client, GaveUp, Refused};
use crate::commands::{Phase, error_json};
use crate::report::{Command, Report, ServerTarget};

#[derive(Parser)]
#[command(name = "dvs-migrate", version, about)]
/// Moves a dvs project's history from local storage onto dvs-server
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Walk every commit on every branch and tag and summarize what a run would do; sends nothing
    Plan(Args),
    /// Walk again, upload the content the server lacks and record who added each version
    ///
    /// The server keeps each run's report on the project's Migration page. Run again until
    /// nothing new is sent, then make local storage read-only.
    Run(RunArgs),
    /// List the blobs in the stores the repos' histories name that no commit references;
    /// reads only, needs no key or server
    Orphans(orphans::Args),
}

#[derive(clap::Args)]
struct RunArgs {
    #[command(flatten)]
    args: Args,
    /// Run although none of the walked commits is in the project's earlier runs
    #[arg(long)]
    other_source: bool,
}

#[derive(clap::Args)]
struct Args {
    /// The project's git repo: a local clone (its branches and its remotes are walked) or a URL
    repo: String,
    /// dvs-server URL
    #[arg(long, value_name = "URL")]
    server: Option<Url>,
    /// Project name on the server
    #[arg(long, value_name = "NAME")]
    project: Option<String>,
    /// Migrate key from the project's Migration page
    #[arg(long, env = "DVS_MIGRATE_KEY", hide_env_values = true)]
    key: String,
    /// A storage path as written in the project's config, and where it is now (repeatable)
    #[arg(long = "storage-map", value_name = "OLD=NEW", value_parser = parse_mapping)]
    storage_map: Vec<(String, String)>,
    /// Print the full report as JSON instead of the summary
    #[arg(long)]
    json: bool,
}

pub(crate) fn parse_mapping(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((old, new)) if !old.is_empty() && !new.is_empty() => {
            Ok((old.to_string(), new.to_string()))
        }
        _ => Err(format!("expected OLD=NEW, got {s:?}")),
    }
}

struct Settings {
    client: Client,
    target: ServerTarget,
    storage_map: BTreeMap<String, String>,
}

fn usage(message: &str) -> anyhow::Error {
    coded(ErrorCode::InvalidArguments, message)
}

fn settings(args: &Args) -> Result<Settings> {
    let server = args
        .server
        .clone()
        .ok_or_else(|| usage("no server: pass --server"))?;
    let project = args
        .project
        .clone()
        .ok_or_else(|| usage("no project: pass --project"))?;
    if !args.key.starts_with("dvsm_") {
        return Err(usage(
            "not a migrate key: issue one on the project's Migration page (it starts with dvsm_)",
        ));
    }
    Ok(Settings {
        client: Client::new(server.clone(), project.clone(), args.key.clone()),
        target: ServerTarget { server, project },
        storage_map: args.storage_map.iter().cloned().collect(),
    })
}

fn execute(
    command: Command,
    args: &Args,
    other_source: bool,
    settings: &Settings,
    started: Started,
    report: &mut Report,
) -> Result<()> {
    let quiet = args.json;
    let plan = commands::plan(&args.repo, &settings.storage_map, &settings.client, quiet);
    if command == Command::Run && plan.as_ref().is_err_and(gave_up) {
        eprintln!("{}", commands::nothing_started());
    }
    let mut plan = plan?;
    match command {
        Command::Plan => {
            if let Some(why) = plan.different_source() {
                eprintln!(
                    "warning: {why}; `dvs-migrate run` refuses it unless --other-source is passed"
                );
            }
            plan.fill(report, Phase::Plan);
        }
        Command::Run => {
            let ran = commands::run(&mut plan, &settings.client, report, quiet, other_source)
                .and_then(|()| {
                    stamp(report, started);
                    settings.client.report(report.for_server())
                });
            if ran.as_ref().is_err_and(gave_up) {
                eprintln!("{}", plan.interrupted());
            }
            ran?;
        }
    }
    Ok(())
}

/// A call's retries ran out (R18)
fn gave_up(error: &anyhow::Error) -> bool {
    error.downcast_ref::<GaveUp>().is_some()
}

/// The exit status: 2 for a usage error or a refusal a flag answers, 1 for any other
/// failure, retries running out included
fn exit_code(result: &Result<()>, printed: bool) -> i32 {
    match result {
        Err(e) if gave_up(e) => 1,
        Err(e) if dvs::backend_error_code(e) == Some(ErrorCode::InvalidArguments) => 2,
        Err(e) if e.downcast_ref::<Refused>().is_some() => 2,
        Err(_) => 1,
        Ok(()) if !printed => 1,
        Ok(()) => 0,
    }
}

/// When the command started, for the report's times
#[derive(Clone, Copy)]
struct Started {
    clock: Instant,
    time: SystemTime,
}

fn stamp(report: &mut Report, started: Started) {
    report.started_at = rfc3339_ms(started.time);
    report.finished_at = rfc3339_ms(SystemTime::now());
    report.duration_ms = duration_ms(started.clock);
}

fn main() {
    let started = Started {
        clock: Instant::now(),
        time: SystemTime::now(),
    };
    git::clean_up_on_signal();
    let cli = Cli::parse();
    let (command, args, other_source) = match &cli.command {
        Cmd::Plan(args) => (Command::Plan, args, false),
        Cmd::Run(run) => (Command::Run, &run.args, run.other_source),
        Cmd::Orphans(args) => std::process::exit(orphans::main(args)),
    };

    let mut report = Report::new(command);
    let (result, target) = match settings(args) {
        Ok(s) => (
            execute(command, args, other_source, &s, started, &mut report),
            Some(s.target),
        ),
        Err(e) => (Err(e), None),
    };
    if let Err(error) = &result {
        report.outcome = dvs::RunOutcome::Failed;
        report.error = Some(error_json(error));
    }
    stamp(&mut report, started);

    let source = git::redact(&args.repo);
    let stdout = &mut std::io::stdout().lock();
    let printed = report
        .print(args.json, &source, target.as_ref(), stdout)
        .and_then(|()| stdout.flush());
    if let Err(e) = &printed {
        eprintln!("error: {e}");
    }
    for warning in report.warnings() {
        eprintln!("{warning}");
    }
    if command == Command::Run && result.is_ok() {
        let scope = report.scope.as_ref().expect("a run fills the scope");
        if let Some(warning) = commands::storage_writable_warning(scope) {
            eprintln!("{warning}");
        }
    }

    std::process::exit(exit_code(&result, printed.is_ok()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_exits_2_and_retries_running_out_exit_1() {
        let refused = Err(Refused("run again with --other-source".into()).into());
        assert_eq!(exit_code(&refused, true), 2);
        let gave_up = Err(client::tests::gave_up());
        assert_eq!(exit_code(&gave_up, true), 1);
        assert_eq!(exit_code(&Err(usage("no server")), true), 2);
        assert_eq!(exit_code(&Ok(()), false), 1);
        assert_eq!(exit_code(&Ok(()), true), 0);
    }
}
