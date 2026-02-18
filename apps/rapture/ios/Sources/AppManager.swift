import Foundation
import Observation

@MainActor
@Observable
final class AppManager: AppReconciler {
    let rust: FfiApp
    var state: AppState
    private var lastRevApplied: UInt64

    init() {
        let fm = FileManager.default
        let dataDirUrl = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let dataDir = dataDirUrl.path
        try? fm.createDirectory(at: dataDirUrl, withIntermediateDirectories: true)

        let rust = FfiApp(dataDir: dataDir)
        self.rust = rust

        let initial = rust.state()
        self.state = initial
        self.lastRevApplied = initial.rev

        rust.listenForUpdates(reconciler: self)
    }

    nonisolated func reconcile(update: AppUpdate) {
        Task { @MainActor [weak self] in
            self?.apply(update: update)
        }
    }

    private func apply(update: AppUpdate) {
        switch update {
        case .fullState(let s):
            if s.rev <= lastRevApplied { return }
            lastRevApplied = s.rev
            state = s
        }
    }

    func dispatch(_ action: AppAction) {
        rust.dispatch(action: action)
    }
}
