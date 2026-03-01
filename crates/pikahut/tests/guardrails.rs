use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

#[test]
fn ci_just_recipes_use_selector_contracts() -> Result<()> {
    let justfile = fs::read_to_string(workspace_root().join("justfile"))?;

    let required = [
        "cargo test -p pikahut --test integration_deterministic cli_smoke_local -- --ignored --nocapture",
        "cargo test -p pikahut --test integration_deterministic openclaw_scenario_invite_and_chat -- --ignored --nocapture",
        "cargo test -p pikahut --test integration_openclaw openclaw_gateway_e2e -- --ignored --nocapture",
        "cargo test -p pikahut --test integration_public ui_e2e_public_all -- --ignored --nocapture",
        "cargo test -p pikahut --test integration_public deployed_bot_call_flow -- --ignored --nocapture",
        "cargo test -p pikahut --test integration_primal primal_nostrconnect_smoke -- --ignored --nocapture",
    ];

    for needle in required {
        assert!(
            justfile.contains(needle),
            "missing required selector contract: {needle}"
        );
    }

    let forbidden = [
        "cargo run -q -p pikahut -- test openclaw-e2e",
        "cargo run -q -p pikahut -- test ui-e2e-local --platform android",
        "cargo run -q -p pikahut -- test ui-e2e-local --platform ios",
        "cargo run -q -p pikahut -- test cli-smoke",
        "./tools/ui-e2e-public --platform all",
    ];

    for needle in forbidden {
        assert!(
            !justfile.contains(needle),
            "forbidden non-selector integration path still present: {needle}"
        );
    }

    Ok(())
}

#[test]
fn integration_wrapper_scripts_dispatch_to_selectors() -> Result<()> {
    let root = workspace_root();

    let ui_public = fs::read_to_string(root.join("tools/ui-e2e-public"))?;
    assert!(ui_public.contains("--test integration_public ui_e2e_public_all"));
    assert!(ui_public.contains("--test integration_public ui_e2e_public_ios"));
    assert!(ui_public.contains("--test integration_public ui_e2e_public_android"));

    let ui_local = fs::read_to_string(root.join("tools/ui-e2e-local"))?;
    assert!(ui_local.contains("--test integration_deterministic ui_e2e_local_ios"));
    assert!(ui_local.contains("--test integration_deterministic ui_e2e_local_android"));
    assert!(ui_local.contains("--test integration_deterministic ui_e2e_local_desktop"));

    let cli_smoke = fs::read_to_string(root.join("tools/cli-smoke"))?;
    assert!(cli_smoke.contains("--test integration_deterministic cli_smoke_local"));

    Ok(())
}
