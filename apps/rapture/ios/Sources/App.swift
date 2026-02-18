import SwiftUI

@main
struct RaptureApp: App {
    @State private var manager = AppManager()

    var body: some Scene {
        WindowGroup {
            ContentView(manager: manager)
        }
    }
}
