use anyhow::Result;

use pikahut::test_harness::OpenclawE2eArgs;
use pikahut::testing::{ArtifactPolicy, Capabilities, Requirement, scenarios};

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

#[tokio::test]
#[ignore = "heavy integration lane (OpenClaw checkout + network)"]
async fn openclaw_gateway_e2e() -> Result<()> {
    let caps = Capabilities::probe(&workspace_root());
    if let Err(skip) =
        caps.require_all_or_skip(&[Requirement::OpenclawCheckout, Requirement::PublicNetwork])
    {
        eprintln!("SKIP: {skip}");
        return Ok(());
    }

    let mut context = pikahut::testing::TestContext::builder("openclaw-gateway-e2e")
        .artifact_policy(ArtifactPolicy::PreserveOnFailure)
        .build()?;

    let result = scenarios::run_openclaw_e2e(OpenclawE2eArgs {
        state_dir: Some(context.state_dir().to_path_buf()),
        relay_url: None,
        openclaw_dir: None,
        keep_state: false,
    })
    .await;

    if result.is_ok() {
        context.mark_success();
    } else {
        eprintln!(
            "openclaw e2e failed; preserved artifacts at {}",
            context.state_dir().join("artifacts/openclaw-e2e").display()
        );
    }

    result
}
