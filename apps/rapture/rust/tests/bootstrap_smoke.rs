use std::time::{Duration, Instant};

use rapture_core::{AppAction, FfiApp};
use tempfile::tempdir;

fn wait_for(timeout: Duration, mut pred: impl FnMut() -> bool) {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("condition not met within {timeout:?}");
}

#[test]
fn ffi_app_bootstrap_and_set_name() {
    let dir = tempdir().expect("tempdir");
    let app = FfiApp::new(dir.path().to_string_lossy().to_string());

    let initial = app.state();
    assert_eq!(initial.rev, 0);
    assert_eq!(initial.greeting, "Rapture ready");

    app.dispatch(AppAction::SetName {
        name: "Rapture".to_string(),
    });

    wait_for(Duration::from_secs(2), || app.state().rev >= 1);
    let updated = app.state();
    assert_eq!(updated.rev, 1);
    assert_eq!(updated.greeting, "Rapture ready, Rapture");
}
