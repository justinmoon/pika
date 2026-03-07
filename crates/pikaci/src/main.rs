use anyhow::{Context, bail};
use clap::{Parser, Subcommand, ValueEnum};
use pikaci::{GuestCommand, JobSpec, LogKind, RunOptions, list_runs, load_logs, run_jobs};

#[derive(Parser, Debug)]
#[command(name = "pikaci")]
#[command(about = "Wave 1 local-first CI runner for Pika")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Run {
        #[arg(default_value = "beachhead")]
        job: String,
    },
    List,
    Logs {
        run_id: String,
        #[arg(long)]
        job: Option<String>,
        #[arg(long, value_enum, default_value = "both")]
        kind: LogKindArg,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum LogKindArg {
    Host,
    Guest,
    Both,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().context("read current directory")?;
    let options = RunOptions {
        source_root: cwd.clone(),
        state_root: cwd.join(".pikaci"),
    };

    match cli.command {
        Command::Run { job } => {
            let specs = run_spec(&job)?;
            let run = run_jobs(specs.as_slice(), &options)?;
            for job in &run.jobs {
                println!(
                    "{} {} {}",
                    run.run_id,
                    job.id,
                    match job.status {
                        pikaci::RunStatus::Passed => "passed",
                        pikaci::RunStatus::Failed => "failed",
                        pikaci::RunStatus::Running => "running",
                    }
                );
                if let Some(message) = &job.message {
                    eprintln!("{message}");
                }
            }
            if matches!(run.status, pikaci::RunStatus::Failed) {
                std::process::exit(1);
            }
        }
        Command::List => {
            for run in list_runs(&options.state_root)? {
                let status = match run.status {
                    pikaci::RunStatus::Running => "running",
                    pikaci::RunStatus::Passed => "passed",
                    pikaci::RunStatus::Failed => "failed",
                };
                let job = run.jobs.first();
                let job_id = job.map(|job| job.id.as_str()).unwrap_or("-");
                println!("{}\t{}\t{}\t{}", run.run_id, status, job_id, run.created_at);
            }
        }
        Command::Logs { run_id, job, kind } => {
            let logs = load_logs(
                &options.state_root,
                &run_id,
                job.as_deref(),
                map_log_kind(kind),
            )?;
            if let Some(host) = logs.host {
                println!("== host ==\n{host}");
            }
            if let Some(guest) = logs.guest {
                println!("== guest ==\n{guest}");
            }
        }
    }

    Ok(())
}

fn run_spec(name: &str) -> anyhow::Result<Vec<JobSpec>> {
    match name {
        "beachhead" => Ok(vec![JobSpec {
            id: "beachhead",
            description: "Run one tiny exact unit test in a vfkit guest",
            timeout_secs: 1800,
            guest_command: GuestCommand::ExactCargoTest {
                package: "pika-agent-control-plane",
                test_name: "tests::command_envelope_round_trips",
            },
        }]),
        "agent-control-plane-unit" => Ok(vec![JobSpec {
            id: "agent-control-plane-unit",
            description: "Run all pika-agent-control-plane unit tests in a vfkit guest",
            timeout_secs: 1800,
            guest_command: GuestCommand::PackageUnitTests {
                package: "pika-agent-control-plane",
            },
        }]),
        "agent-microvm-tests" => Ok(vec![JobSpec {
            id: "agent-microvm-tests",
            description: "Run pika-agent-microvm tests in a vfkit guest",
            timeout_secs: 1800,
            guest_command: GuestCommand::PackageTests {
                package: "pika-agent-microvm",
            },
        }]),
        "server-agent-api-tests" => Ok(vec![JobSpec {
            id: "server-agent-api-tests",
            description: "Run pika-server agent_api tests in a vfkit guest",
            timeout_secs: 1800,
            guest_command: GuestCommand::FilteredCargoTests {
                package: "pika-server",
                filter: "agent_api::tests",
            },
        }]),
        "core-agent-nip98-test" => Ok(vec![JobSpec {
            id: "core-agent-nip98-test",
            description: "Run pika_core NIP-98 signing contract test in a vfkit guest",
            timeout_secs: 1800,
            guest_command: GuestCommand::ExactCargoTest {
                package: "pika_core",
                test_name: "core::agent::tests::run_agent_flow_signs_requests_with_nip98_authorization",
            },
        }]),
        "agent-contracts-smoke" => Ok(vec![
            JobSpec {
                id: "agent-control-plane-unit",
                description: "Run all pika-agent-control-plane unit tests in a vfkit guest",
                timeout_secs: 1800,
                guest_command: GuestCommand::PackageUnitTests {
                    package: "pika-agent-control-plane",
                },
            },
            JobSpec {
                id: "agent-microvm-tests",
                description: "Run pika-agent-microvm tests in a vfkit guest",
                timeout_secs: 1800,
                guest_command: GuestCommand::PackageTests {
                    package: "pika-agent-microvm",
                },
            },
            JobSpec {
                id: "server-agent-api-tests",
                description: "Run pika-server agent_api tests in a vfkit guest",
                timeout_secs: 1800,
                guest_command: GuestCommand::FilteredCargoTests {
                    package: "pika-server",
                    filter: "agent_api::tests",
                },
            },
            JobSpec {
                id: "core-agent-nip98-test",
                description: "Run pika_core NIP-98 signing contract test in a vfkit guest",
                timeout_secs: 1800,
                guest_command: GuestCommand::ExactCargoTest {
                    package: "pika_core",
                    test_name: "core::agent::tests::run_agent_flow_signs_requests_with_nip98_authorization",
                },
            },
        ]),
        other => bail!("unknown job `{other}`"),
    }
}

fn map_log_kind(kind: LogKindArg) -> LogKind {
    match kind {
        LogKindArg::Host => LogKind::Host,
        LogKindArg::Guest => LogKind::Guest,
        LogKindArg::Both => LogKind::Both,
    }
}
