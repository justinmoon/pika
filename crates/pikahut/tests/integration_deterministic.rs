use std::path::{Path, PathBuf};

use anyhow::Result;

use pikahut::test_harness::{
    CliSmokeArgs, InteropRustBaselineArgs, TestScenarioArgs, UiE2eLocalArgs, UiPlatform,
};
use pikahut::testing::{Capabilities, Requirement, scenarios};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn capabilities() -> Capabilities {
    Capabilities::probe(&workspace_root())
}

fn skip_if_missing(requirements: &[Requirement]) -> Result<bool> {
    let caps = capabilities();
    match caps.require_all_or_skip(requirements) {
        Ok(()) => Ok(false),
        Err(skip) => {
            eprintln!("SKIP: {skip}");
            Ok(true)
        }
    }
}

#[tokio::test]
#[ignore = "integration scenario; run in deterministic lane"]
async fn cli_smoke_local() -> Result<()> {
    scenarios::run_cli_smoke(CliSmokeArgs {
        relay: None,
        with_media: false,
        state_dir: None,
    })
    .await
}

#[tokio::test]
#[ignore = "integration scenario; run in deterministic lane with network"]
async fn cli_smoke_media_local() -> Result<()> {
    if skip_if_missing(&[Requirement::PublicNetwork])? {
        return Ok(());
    }

    scenarios::run_cli_smoke(CliSmokeArgs {
        relay: None,
        with_media: true,
        state_dir: None,
    })
    .await
}

#[tokio::test]
#[ignore = "requires Android SDK/emulator"]
async fn ui_e2e_local_android() -> Result<()> {
    if skip_if_missing(&[Requirement::AndroidTools, Requirement::AndroidEmulatorAvd])? {
        return Ok(());
    }

    scenarios::run_ui_e2e_local(UiE2eLocalArgs {
        platform: UiPlatform::Android,
        state_dir: None,
        keep: false,
        bot_timeout_sec: None,
    })
    .await
}

#[tokio::test]
#[ignore = "requires macOS + Xcode"]
async fn ui_e2e_local_ios() -> Result<()> {
    if skip_if_missing(&[Requirement::HostMacOs, Requirement::Xcode])? {
        return Ok(());
    }

    scenarios::run_ui_e2e_local(UiE2eLocalArgs {
        platform: UiPlatform::Ios,
        state_dir: None,
        keep: false,
        bot_timeout_sec: None,
    })
    .await
}

#[tokio::test]
#[ignore = "desktop UI e2e can be heavy in CI"]
async fn ui_e2e_local_desktop() -> Result<()> {
    scenarios::run_ui_e2e_local(UiE2eLocalArgs {
        platform: UiPlatform::Desktop,
        state_dir: None,
        keep: false,
        bot_timeout_sec: None,
    })
    .await
}

#[tokio::test]
#[ignore = "requires external rust interop repo"]
async fn interop_rust_baseline() -> Result<()> {
    if skip_if_missing(&[Requirement::InteropRustRepo])? {
        return Ok(());
    }

    scenarios::run_interop_rust_baseline(InteropRustBaselineArgs {
        manual: false,
        keep: false,
        state_dir: None,
        rust_interop_dir: None,
        bot_timeout_sec: None,
    })
    .await
}

#[tokio::test]
#[ignore = "deterministic OpenClaw scenario"]
async fn openclaw_scenario_invite_and_chat() -> Result<()> {
    scenarios::run_scenario(TestScenarioArgs {
        scenario: "invite-and-chat".to_string(),
        state_dir: None,
        relay: None,
        extra_args: Vec::new(),
    })
    .await
}

#[tokio::test]
#[ignore = "deterministic OpenClaw scenario"]
async fn openclaw_scenario_invite_and_chat_rust_bot() -> Result<()> {
    scenarios::run_scenario(TestScenarioArgs {
        scenario: "invite-and-chat-rust-bot".to_string(),
        state_dir: None,
        relay: None,
        extra_args: Vec::new(),
    })
    .await
}

#[tokio::test]
#[ignore = "deterministic OpenClaw scenario"]
async fn openclaw_scenario_invite_and_chat_daemon() -> Result<()> {
    scenarios::run_scenario(TestScenarioArgs {
        scenario: "invite-and-chat-daemon".to_string(),
        state_dir: None,
        relay: None,
        extra_args: Vec::new(),
    })
    .await
}

#[tokio::test]
#[ignore = "deterministic OpenClaw scenario"]
async fn openclaw_scenario_audio_echo() -> Result<()> {
    scenarios::run_scenario(TestScenarioArgs {
        scenario: "audio-echo".to_string(),
        state_dir: None,
        relay: None,
        extra_args: Vec::new(),
    })
    .await
}
