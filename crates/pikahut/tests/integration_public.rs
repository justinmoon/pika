use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use pikahut::testing::{
    ArtifactPolicy, Capabilities, CommandRunner, CommandSpec, Requirement, TestContext,
};

const ENV_PIKA_TEST_NSEC: &str = "PIKA_TEST_NSEC";
const ENV_PIKA_UI_E2E_BOT_NPUB: &str = "PIKA_UI_E2E_BOT_NPUB";
const ENV_PIKA_UI_E2E_RELAYS: &str = "PIKA_UI_E2E_RELAYS";
const ENV_PIKA_UI_E2E_KP_RELAYS: &str = "PIKA_UI_E2E_KP_RELAYS";
const ENV_PIKA_UI_E2E_NSEC: &str = "PIKA_UI_E2E_NSEC";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn skip_if_missing(requirements: &[Requirement]) -> Result<bool> {
    let caps = Capabilities::probe(&workspace_root());
    match caps.require_all_or_skip(requirements) {
        Ok(()) => Ok(false),
        Err(skip) => {
            eprintln!("SKIP: {skip}");
            Ok(true)
        }
    }
}

fn required_env(name: &'static str) -> Result<String> {
    std::env::var(name)
        .with_context(|| format!("missing required env: {name}"))
        .map(|value| value.trim().to_string())
        .and_then(|value| {
            if value.is_empty() {
                bail!("required env is empty: {name}");
            }
            Ok(value)
        })
}

fn optional_env(name: &'static str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_udid(output: &str) -> Option<String> {
    for line in output.lines() {
        let prefix = "ok: ios simulator ready (udid=";
        if let Some(rest) = line.strip_prefix(prefix)
            && let Some(udid) = rest.strip_suffix(')')
        {
            return Some(udid.to_string());
        }
    }
    None
}

fn run_public_android(runner: &CommandRunner<'_>, root: &Path) -> Result<()> {
    let peer = required_env(ENV_PIKA_UI_E2E_BOT_NPUB)?;
    let relays = required_env(ENV_PIKA_UI_E2E_RELAYS)?;
    let kp_relays = required_env(ENV_PIKA_UI_E2E_KP_RELAYS)?;
    let nsec = optional_env(ENV_PIKA_UI_E2E_NSEC)
        .or_else(|| optional_env(ENV_PIKA_TEST_NSEC))
        .ok_or_else(|| anyhow!("missing {ENV_PIKA_UI_E2E_NSEC} and {ENV_PIKA_TEST_NSEC}"))?;

    let test_suffix = optional_env("PIKA_ANDROID_TEST_APPLICATION_ID_SUFFIX")
        .unwrap_or_else(|| ".test".to_string());
    let test_app_id = format!("org.pikachat.pika{test_suffix}");

    if optional_env("PIKA_ANDROID_SERIAL").is_none() {
        runner.run(
            &CommandSpec::new("./tools/android-emulator-ensure")
                .cwd(root)
                .capture_name("android-emulator-ensure"),
        )?;
    }

    runner.run(
        &CommandSpec::new("just")
            .cwd(root)
            .args(["gen-kotlin", "android-rust", "android-local-properties"])
            .capture_name("android-prepare-build"),
    )?;

    runner.run(
        &CommandSpec::new("./tools/android-ensure-debug-installable")
            .cwd(root)
            .env("PIKA_ANDROID_APP_ID", &test_app_id)
            .capture_name("android-ensure-installable"),
    )?;

    let serial_output = runner.run(
        &CommandSpec::new("./tools/android-pick-serial")
            .cwd(root)
            .capture_name("android-pick-serial"),
    )?;
    let serial = String::from_utf8_lossy(&serial_output.stdout)
        .trim()
        .to_string();
    if serial.is_empty() {
        bail!("android serial output was empty");
    }

    if !serial.starts_with("emulator-") {
        runner.run(
            &CommandSpec::new("./tools/android-ensure-unlocked")
                .cwd(root)
                .arg(serial.clone())
                .capture_name("android-ensure-unlocked"),
        )?;
    }

    runner.run(
        &CommandSpec::gradlew()
            .cwd(root.join("android"))
            .env("ANDROID_SERIAL", serial)
            .arg(":app:connectedDebugAndroidTest")
            .arg(format!("-PPIKA_ANDROID_APPLICATION_ID_SUFFIX={test_suffix}"))
            .arg("-Pandroid.testInstrumentationRunnerArguments.class=com.pika.app.PikaE2eUiTest")
            .arg("-Pandroid.testInstrumentationRunnerArguments.pika_e2e=1")
            .arg("-Pandroid.testInstrumentationRunnerArguments.pika_disable_network=false")
            .arg("-Pandroid.testInstrumentationRunnerArguments.pika_reset=1")
            .arg(format!("-Pandroid.testInstrumentationRunnerArguments.pika_peer_npub={peer}"))
            .arg(format!("-Pandroid.testInstrumentationRunnerArguments.pika_relay_urls={relays}"))
            .arg(format!("-Pandroid.testInstrumentationRunnerArguments.pika_key_package_relay_urls={kp_relays}"))
            .arg(format!("-Pandroid.testInstrumentationRunnerArguments.pika_nsec={nsec}"))
            .capture_name("android-ui-e2e-public"),
    )?;

    Ok(())
}

fn run_public_ios(runner: &CommandRunner<'_>, root: &Path) -> Result<()> {
    let peer = required_env(ENV_PIKA_UI_E2E_BOT_NPUB)?;
    let relays = required_env(ENV_PIKA_UI_E2E_RELAYS)?;
    let kp_relays = required_env(ENV_PIKA_UI_E2E_KP_RELAYS)?;
    let nsec = optional_env(ENV_PIKA_UI_E2E_NSEC)
        .or_else(|| optional_env(ENV_PIKA_TEST_NSEC))
        .ok_or_else(|| anyhow!("missing {ENV_PIKA_UI_E2E_NSEC} and {ENV_PIKA_TEST_NSEC}"))?;

    runner.run(
        &CommandSpec::new("just")
            .cwd(root)
            .args(["ios-xcframework", "ios-xcodeproj"])
            .capture_name("ios-prepare-build"),
    )?;

    let sim_output = runner.run(
        &CommandSpec::new("./tools/ios-sim-ensure")
            .cwd(root)
            .env(ENV_PIKA_UI_E2E_NSEC, &nsec)
            .capture_name("ios-sim-ensure-public"),
    )?;
    let sim_stdout = String::from_utf8_lossy(&sim_output.stdout);
    let udid = parse_udid(&sim_stdout)
        .ok_or_else(|| anyhow!("could not determine simulator udid from ios-sim-ensure"))?;

    runner.run(
        &CommandSpec::new("./tools/xcode-run")
            .cwd(root)
            .env("PIKA_UI_E2E", "1")
            .env("PIKA_UI_E2E_BOT_NPUB", &peer)
            .env("PIKA_UI_E2E_RELAYS", &relays)
            .env("PIKA_UI_E2E_KP_RELAYS", &kp_relays)
            .env("PIKA_UI_E2E_NSEC", &nsec)
            .arg("xcodebuild")
            .args(["-project", "ios/Pika.xcodeproj", "-scheme", "Pika"])
            .arg("-destination")
            .arg(format!("id={udid}"))
            .arg("test")
            .arg("CODE_SIGNING_ALLOWED=NO")
            .arg(format!(
                "PIKA_APP_BUNDLE_ID={}",
                optional_env("PIKA_IOS_BUNDLE_ID")
                    .unwrap_or_else(|| "org.pikachat.pika.dev".to_string())
            ))
            .arg("-only-testing:PikaUITests/PikaUITests/testE2E_deployedRustBot_pingPong")
            .capture_name("ios-ui-e2e-public"),
    )?;

    Ok(())
}

#[test]
#[ignore = "nondeterministic public relay flow"]
fn ui_e2e_public_android() -> Result<()> {
    if skip_if_missing(&[
        Requirement::PublicNetwork,
        Requirement::EnvSecretPikaTestNsec,
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_BOT_NPUB,
        },
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_RELAYS,
        },
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_KP_RELAYS,
        },
        Requirement::AndroidTools,
        Requirement::AndroidEmulatorAvd,
    ])? {
        return Ok(());
    }

    let mut context = TestContext::builder("ui-e2e-public-android")
        .artifact_policy(ArtifactPolicy::PreserveOnFailure)
        .build()?;
    let root = workspace_root();
    let runner = CommandRunner::new(&context);
    let result = run_public_android(&runner, &root);
    if result.is_ok() {
        context.mark_success();
    }
    result
}

#[test]
#[ignore = "nondeterministic public relay flow"]
fn ui_e2e_public_ios() -> Result<()> {
    if skip_if_missing(&[
        Requirement::PublicNetwork,
        Requirement::EnvSecretPikaTestNsec,
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_BOT_NPUB,
        },
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_RELAYS,
        },
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_KP_RELAYS,
        },
        Requirement::HostMacOs,
        Requirement::Xcode,
    ])? {
        return Ok(());
    }

    let mut context = TestContext::builder("ui-e2e-public-ios")
        .artifact_policy(ArtifactPolicy::PreserveOnFailure)
        .build()?;
    let root = workspace_root();
    let runner = CommandRunner::new(&context);
    let result = run_public_ios(&runner, &root);
    if result.is_ok() {
        context.mark_success();
    }
    result
}

#[test]
#[ignore = "nondeterministic public relay flow"]
fn ui_e2e_public_all() -> Result<()> {
    if skip_if_missing(&[
        Requirement::PublicNetwork,
        Requirement::EnvSecretPikaTestNsec,
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_BOT_NPUB,
        },
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_RELAYS,
        },
        Requirement::EnvVar {
            name: ENV_PIKA_UI_E2E_KP_RELAYS,
        },
        Requirement::HostMacOs,
        Requirement::Xcode,
        Requirement::AndroidTools,
        Requirement::AndroidEmulatorAvd,
    ])? {
        return Ok(());
    }

    let mut context = TestContext::builder("ui-e2e-public-all")
        .artifact_policy(ArtifactPolicy::PreserveOnFailure)
        .build()?;
    let root = workspace_root();
    let runner = CommandRunner::new(&context);
    let result = run_public_ios(&runner, &root).and_then(|_| run_public_android(&runner, &root));
    if result.is_ok() {
        context.mark_success();
    }
    result
}

#[test]
#[ignore = "nondeterministic deployed bot flow"]
fn deployed_bot_call_flow() -> Result<()> {
    if skip_if_missing(&[
        Requirement::PublicNetwork,
        Requirement::EnvSecretPikaTestNsec,
    ])? {
        return Ok(());
    }

    let mut context = TestContext::builder("deployed-bot-call-flow")
        .artifact_policy(ArtifactPolicy::PreserveOnFailure)
        .build()?;
    let root = workspace_root();
    let runner = CommandRunner::new(&context);

    let result = runner.run(
        &CommandSpec::cargo()
            .cwd(&root)
            .args([
                "test",
                "-p",
                "pika_core",
                "--test",
                "e2e_calls",
                "call_deployed_bot",
                "--",
                "--ignored",
                "--nocapture",
            ])
            .capture_name("deployed-bot-call-flow"),
    );

    if result.is_ok() {
        context.mark_success();
    }

    result.map(|_| ())
}
