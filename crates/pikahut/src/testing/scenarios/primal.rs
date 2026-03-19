use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::config;
use crate::testing::{ArtifactPolicy, CommandOutput, CommandRunner, CommandSpec, TestContext};

use super::artifacts::{self, CommandOutcomeRecord};
use super::common::extract_udid;
use super::types::ScenarioRunOutput;

const DEFAULT_PRIMAL_REPO_URL: &str = "https://github.com/PrimalHQ/primal-ios-app.git";
const DEFAULT_PRIMAL_REF: &str = "9788ac5bf8ac5746a4eb2ab7e66d4a1f434c005d";
const DEFAULT_PRIMAL_BUNDLE_ID: &str = "net.primal.iosapp.Primal";
const DEFAULT_PIKA_BUNDLE_ID: &str = "com.justinmoon.pika.dev";

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_or(name: &str, fallback: &str) -> String {
    env_opt(name).unwrap_or_else(|| fallback.to_string())
}

fn resolve_primal_clone_source(primal_repo_url: &str) -> Result<String> {
    if let Some(path) = env_opt("PIKA_PRIMAL_SRC_DIR") {
        let src_dir = PathBuf::from(path);
        if !src_dir.join(".git").is_dir() {
            bail!(
                "PIKA_PRIMAL_SRC_DIR must point to a git repo, got {}",
                src_dir.display()
            );
        }
        return Ok(src_dir.to_string_lossy().to_string());
    }

    Ok(primal_repo_url.to_string())
}

fn record_command(
    name: &str,
    output: &CommandOutput,
    outcomes: &mut Vec<CommandOutcomeRecord>,
    artifacts: &mut Vec<PathBuf>,
) {
    outcomes.push(CommandOutcomeRecord::from_output(name, output));
    artifacts.push(output.stdout_path.clone());
    artifacts.push(output.stderr_path.clone());
}

fn output_stdout_trimmed(output: &CommandOutput) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn find_named_dir(root: &Path, dir_name: &str) -> Result<Option<PathBuf>> {
    let mut queue = vec![root.to_path_buf()];
    while let Some(path) = queue.pop() {
        if !path.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let candidate = entry.path();
            if candidate.is_dir() {
                if entry.file_name().to_string_lossy() == dir_name {
                    return Ok(Some(candidate));
                }
                queue.push(candidate);
            }
        }
    }
    Ok(None)
}

pub fn run_primal_nostrconnect_smoke() -> Result<ScenarioRunOutput> {
    let root = config::find_workspace_root()?;
    let mut context = TestContext::builder("primal-nostrconnect-smoke")
        .artifact_policy(ArtifactPolicy::PreserveOnFailure)
        .build()?;
    let runner = CommandRunner::new(&context);

    let mut command_outcomes = Vec::new();
    let mut artifacts = Vec::new();

    let primal_repo_url = env_or("PIKA_PRIMAL_REPO_URL", DEFAULT_PRIMAL_REPO_URL);
    let primal_ref = env_or("PIKA_PRIMAL_REF", DEFAULT_PRIMAL_REF);
    let primal_bundle_id = env_or("PIKA_PRIMAL_BUNDLE_ID", DEFAULT_PRIMAL_BUNDLE_ID);
    let pika_bundle_id = env_or("PIKA_IOS_BUNDLE_ID", DEFAULT_PIKA_BUNDLE_ID);
    let primal_clone_source = resolve_primal_clone_source(&primal_repo_url)?;
    let primal_checkout_dir = context.state_dir().join("primal/source");

    let parent = primal_checkout_dir.parent().ok_or_else(|| {
        anyhow!(
            "invalid primal checkout path: {}",
            primal_checkout_dir.display()
        )
    })?;
    fs::create_dir_all(parent)?;

    if !primal_checkout_dir.join(".git").is_dir() {
        let clone = runner.run(
            &CommandSpec::new("git")
                .cwd(parent)
                .args(["clone"])
                .arg(primal_clone_source.clone())
                .arg(primal_checkout_dir.to_string_lossy().to_string())
                .capture_name("primal-git-clone"),
        )?;
        record_command(
            "primal-git-clone",
            &clone,
            &mut command_outcomes,
            &mut artifacts,
        );
    }

    let fetch = runner.run(
        &CommandSpec::new("git")
            .cwd(&primal_checkout_dir)
            .args(["fetch", "origin"])
            .arg(primal_ref.clone())
            .capture_name("primal-git-fetch"),
    )?;
    record_command(
        "primal-git-fetch",
        &fetch,
        &mut command_outcomes,
        &mut artifacts,
    );

    let checkout = runner.run(
        &CommandSpec::new("git")
            .cwd(&primal_checkout_dir)
            .args(["checkout", "--detach", "FETCH_HEAD"])
            .capture_name("primal-git-checkout"),
    )?;
    record_command(
        "primal-git-checkout",
        &checkout,
        &mut command_outcomes,
        &mut artifacts,
    );

    for lib_id in ["ios-arm64-simulator", "ios-arm64"] {
        fs::create_dir_all(
            primal_checkout_dir
                .join("Primal/PrimalShared.xcframework")
                .join(lib_id)
                .join("dSYMs"),
        )?;
    }

    let sim_ensure = runner.run(
        &CommandSpec::new("./tools/ios-sim-ensure")
            .cwd(&root)
            .capture_name("primal-ios-sim-ensure"),
    )?;
    record_command(
        "primal-ios-sim-ensure",
        &sim_ensure,
        &mut command_outcomes,
        &mut artifacts,
    );
    let udid = extract_udid(&output_stdout_trimmed(&sim_ensure))
        .ok_or_else(|| anyhow!("could not determine simulator udid from ios-sim-ensure"))?;

    let derived_dir = context.state_dir().join("primal/derived-data");
    fs::create_dir_all(&derived_dir)?;

    let primal_build = runner.run(
        &CommandSpec::new("./tools/xcode-run")
            .cwd(&root)
            .arg("xcodebuild")
            .args(["-project"])
            .arg(
                primal_checkout_dir
                    .join("Primal.xcodeproj")
                    .to_string_lossy()
                    .to_string(),
            )
            .args(["-scheme", "Primal"])
            .args(["-configuration", "Debug"])
            .args(["-sdk", "iphonesimulator"])
            .args(["-destination"])
            .arg(format!("id={udid}"))
            .args(["-derivedDataPath"])
            .arg(derived_dir.to_string_lossy().to_string())
            .arg("build")
            .arg("-skipMacroValidation")
            .arg("CODE_SIGNING_ALLOWED=NO")
            .capture_name("primal-xcodebuild"),
    )?;
    record_command(
        "primal-xcodebuild",
        &primal_build,
        &mut command_outcomes,
        &mut artifacts,
    );

    let primal_app_path = find_named_dir(&derived_dir.join("Build/Products"), "Primal.app")?
        .ok_or_else(|| {
            anyhow!(
                "failed to locate Primal.app under {}",
                derived_dir.join("Build/Products").display()
            )
        })?;

    if let Ok(boot) = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args(["simctl", "boot", &udid])
            .capture_name("primal-sim-boot"),
    ) {
        record_command(
            "primal-sim-boot",
            &boot,
            &mut command_outcomes,
            &mut artifacts,
        );
    }

    let bootstatus = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args(["simctl", "bootstatus", &udid, "-b"])
            .capture_name("primal-sim-bootstatus"),
    )?;
    record_command(
        "primal-sim-bootstatus",
        &bootstatus,
        &mut command_outcomes,
        &mut artifacts,
    );

    let install = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args(["simctl", "install", &udid])
            .arg(primal_app_path.to_string_lossy().to_string())
            .capture_name("primal-sim-install"),
    )?;
    record_command(
        "primal-sim-install",
        &install,
        &mut command_outcomes,
        &mut artifacts,
    );

    if let Ok(launch) = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args(["simctl", "launch", &udid, &primal_bundle_id])
            .capture_name("primal-sim-launch"),
    ) {
        record_command(
            "primal-sim-launch",
            &launch,
            &mut command_outcomes,
            &mut artifacts,
        );
    }
    if let Ok(terminate) = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args(["simctl", "terminate", &udid, &primal_bundle_id])
            .capture_name("primal-sim-terminate"),
    ) {
        record_command(
            "primal-sim-terminate",
            &terminate,
            &mut command_outcomes,
            &mut artifacts,
        );
    }

    let probe_url = "nostrconnect://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?name=Pika&url=https%3A%2F%2Fpika.chat&secret=sec-nightly-probe&relay=wss%3A%2F%2Frelay.primal.net".to_string();
    let probe = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args(["simctl", "openurl", &udid, &probe_url])
            .capture_name("primal-openurl-probe"),
    )?;
    record_command(
        "primal-openurl-probe",
        &probe,
        &mut command_outcomes,
        &mut artifacts,
    );

    let pika_build = runner.run(
        &CommandSpec::new("just")
            .cwd(&root)
            .args(["ios-xcframework", "ios-xcodeproj"])
            .capture_name("pika-ios-build-artifacts"),
    )?;
    record_command(
        "pika-ios-build-artifacts",
        &pika_build,
        &mut command_outcomes,
        &mut artifacts,
    );

    if let Ok(pre_container) = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args([
                "simctl",
                "get_app_container",
                &udid,
                &pika_bundle_id,
                "data",
            ])
            .capture_name("pika-app-data-dir-pre"),
    ) {
        record_command(
            "pika-app-data-dir-pre",
            &pre_container,
            &mut command_outcomes,
            &mut artifacts,
        );
        let pre_data_dir = output_stdout_trimmed(&pre_container);
        if !pre_data_dir.is_empty() {
            let _ =
                fs::remove_file(PathBuf::from(pre_data_dir).join("Documents/ui_test_open_url.txt"));
        }
    }

    let smoke = runner.run(
        &CommandSpec::new("./tools/xcode-run")
            .cwd(&root)
            .env("PIKA_PRIMAL_BUNDLE_ID", primal_bundle_id.clone())
            .arg("xcodebuild")
            .args(["-project", "ios/Pika.xcodeproj", "-scheme", "Pika"])
            .args(["-derivedDataPath", "ios/build"])
            .args(["-destination"])
            .arg(format!("id={udid}"))
            .arg("test")
            .arg("-skipMacroValidation")
            .arg("ARCHS=arm64")
            .arg("ONLY_ACTIVE_ARCH=YES")
            .arg("CODE_SIGNING_ALLOWED=NO")
            .arg(format!("PIKA_APP_BUNDLE_ID={pika_bundle_id}"))
            .arg("-only-testing:PikaUITests/PikaUITests/testInterop_nostrConnectLaunchesPrimal")
            .capture_name("pika-primal-interop-smoke"),
    )?;
    record_command(
        "pika-primal-interop-smoke",
        &smoke,
        &mut command_outcomes,
        &mut artifacts,
    );

    let data_container = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args([
                "simctl",
                "get_app_container",
                &udid,
                &pika_bundle_id,
                "data",
            ])
            .capture_name("pika-app-data-dir"),
    )?;
    record_command(
        "pika-app-data-dir",
        &data_container,
        &mut command_outcomes,
        &mut artifacts,
    );
    let data_dir = output_stdout_trimmed(&data_container);
    if data_dir.is_empty() {
        bail!("xcrun simctl get_app_container returned empty data path");
    }

    let marker_file = PathBuf::from(&data_dir).join("Documents/ui_test_open_url.txt");
    if !marker_file.is_file() {
        bail!("missing marker file: {}", marker_file.display());
    }
    let marker_content = fs::read_to_string(&marker_file)
        .with_context(|| format!("read {}", marker_file.display()))?;
    if !marker_content.contains("nostrconnect://") {
        bail!(
            "marker file missing nostrconnect URL: {}",
            marker_file.display()
        );
    }
    if !marker_content.contains("?secret=") && !marker_content.contains("&secret=") {
        bail!(
            "marker file missing secret query param: {}",
            marker_file.display()
        );
    }
    if !marker_content.contains("?callback=") && !marker_content.contains("&callback=") {
        bail!(
            "marker file missing callback query param: {}",
            marker_file.display()
        );
    }

    if let Ok(sim_log) = runner.run(
        &CommandSpec::new("xcrun")
            .cwd(&root)
            .args(["simctl", "spawn", &udid, "log", "show"])
            .args(["--style", "compact", "--last", "20m"])
            .args([
                "--predicate",
                "process == \"Pika\" OR process == \"Primal\" OR composedMessage CONTAINS[c] \"nostrconnect\" OR composedMessage CONTAINS[c] \"PikaSignerBridge\"",
            ])
            .capture_name("primal-simulator-log"),
    ) {
        record_command(
            "primal-simulator-log",
            &sim_log,
            &mut command_outcomes,
            &mut artifacts,
        );
    }

    let probe_artifact =
        context.write_artifact("primal-nightly/probe_url.txt", format!("{probe_url}\n"))?;
    let data_dir_artifact =
        context.write_artifact("primal-nightly/data_dir.txt", format!("{data_dir}\n"))?;
    let marker_artifact =
        context.write_artifact("primal-nightly/ui_test_open_url.txt", marker_content)?;

    let mut result = ScenarioRunOutput::completed(context.state_dir().to_path_buf())
        .with_artifact(probe_artifact)
        .with_artifact(data_dir_artifact)
        .with_artifact(marker_artifact)
        .with_metadata("primal_repo_url", primal_repo_url)
        .with_metadata("primal_ref", primal_ref)
        .with_metadata("primal_clone_source", primal_clone_source)
        .with_metadata(
            "primal_checkout_dir",
            primal_checkout_dir.to_string_lossy().to_string(),
        )
        .with_metadata("primal_bundle_id", primal_bundle_id)
        .with_metadata("pika_bundle_id", pika_bundle_id)
        .with_metadata("simulator_udid", udid);

    for artifact in artifacts {
        result = result.with_artifact(artifact);
    }

    let summary = artifacts::write_standard_summary(
        &context,
        "primal::nostrconnect_smoke",
        &result,
        command_outcomes,
        BTreeMap::new(),
    )?;
    result = result.with_summary(summary);

    context.mark_success();
    Ok(result)
}
