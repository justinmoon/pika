use anyhow::Result;

use pikahut::testing::{
    ArtifactPolicy, Capabilities, CommandRunner, CommandSpec, Requirement, TestContext,
};

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

#[test]
#[ignore = "nightly macOS primal interop lane"]
fn primal_nostrconnect_smoke() -> Result<()> {
    let caps = Capabilities::probe(&workspace_root());
    if let Err(skip) = caps.require_all_or_skip(&[
        Requirement::HostMacOs,
        Requirement::Xcode,
        Requirement::PublicNetwork,
    ]) {
        eprintln!("SKIP: {skip}");
        return Ok(());
    }

    let mut context = TestContext::builder("primal-nostrconnect-smoke")
        .artifact_policy(ArtifactPolicy::PreserveOnFailure)
        .build()?;
    let runner = CommandRunner::new(&context);

    let artifact_dir = context.ensure_artifact_subdir("primal-nightly")?;
    let result = runner.run(
        &CommandSpec::new("./tools/primal-ios-interop-nightly")
            .cwd(workspace_root())
            .env(
                "PIKA_PRIMAL_ARTIFACT_DIR",
                artifact_dir.to_string_lossy().to_string(),
            )
            .capture_name("primal-nightly-smoke"),
    );

    if result.is_ok() {
        context.mark_success();
    }

    result.map(|_| ())
}
