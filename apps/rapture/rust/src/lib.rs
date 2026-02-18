use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use flume::{Receiver, Sender};

uniffi::setup_scaffolding!();

// ── State ───────────────────────────────────────────────────────────────────

#[derive(uniffi::Record, Clone, Debug)]
pub struct AppState {
    pub rev: u64,
    pub greeting: String,
}

impl AppState {
    fn empty() -> Self {
        Self {
            rev: 0,
            greeting: "Hello from Rust! 🦀".to_string(),
        }
    }
}

// ── Actions & Updates ───────────────────────────────────────────────────────

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AppAction {
    SetName { name: String },
}

#[derive(uniffi::Enum, Clone, Debug)]
pub enum AppUpdate {
    FullState(AppState),
}

// ── Callback interface ──────────────────────────────────────────────────────

#[uniffi::export(callback_interface)]
pub trait AppReconciler: Send + Sync + 'static {
    fn reconcile(&self, update: AppUpdate);
}

// ── FFI entry point ─────────────────────────────────────────────────────────

enum CoreMsg {
    Action(AppAction),
}

#[derive(uniffi::Object)]
pub struct FfiApp {
    core_tx: Sender<CoreMsg>,
    update_rx: Receiver<AppUpdate>,
    listening: AtomicBool,
    shared_state: Arc<RwLock<AppState>>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Arc<Self> {
        let _ = data_dir; // reserved for future use

        let (update_tx, update_rx) = flume::unbounded();
        let (core_tx, core_rx) = flume::unbounded::<CoreMsg>();
        let shared_state = Arc::new(RwLock::new(AppState::empty()));

        let shared_for_core = shared_state.clone();
        thread::spawn(move || {
            let mut state = AppState::empty();
            let mut rev: u64 = 0;

            // Emit initial state.
            {
                let snapshot = state.clone();
                match shared_for_core.write() {
                    Ok(mut g) => *g = snapshot.clone(),
                    Err(p) => *p.into_inner() = snapshot.clone(),
                }
                let _ = update_tx.send(AppUpdate::FullState(snapshot));
            }

            while let Ok(msg) = core_rx.recv() {
                match msg {
                    CoreMsg::Action(action) => match action {
                        AppAction::SetName { name } => {
                            rev += 1;
                            state.rev = rev;
                            if name.trim().is_empty() {
                                state.greeting = "Hello from Rust! 🦀".to_string();
                            } else {
                                state.greeting = format!("Hello, {}! 🦀", name.trim());
                            }
                            let snapshot = state.clone();
                            match shared_for_core.write() {
                                Ok(mut g) => *g = snapshot.clone(),
                                Err(p) => *p.into_inner() = snapshot.clone(),
                            }
                            let _ = update_tx.send(AppUpdate::FullState(snapshot));
                        }
                    },
                }
            }
        });

        Arc::new(Self {
            core_tx,
            update_rx,
            listening: AtomicBool::new(false),
            shared_state,
        })
    }

    pub fn state(&self) -> AppState {
        match self.shared_state.read() {
            Ok(g) => g.clone(),
            Err(poison) => poison.into_inner().clone(),
        }
    }

    pub fn dispatch(&self, action: AppAction) {
        let _ = self.core_tx.send(CoreMsg::Action(action));
    }

    pub fn listen_for_updates(&self, reconciler: Box<dyn AppReconciler>) {
        if self
            .listening
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let rx = self.update_rx.clone();
        thread::spawn(move || {
            while let Ok(update) = rx.recv() {
                reconciler.reconcile(update);
            }
        });
    }
}
