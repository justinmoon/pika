import SwiftUI

struct ContentView: View {
    @Bindable var manager: AppManager
    @State private var nameInput = ""

    var body: some View {
        VStack(spacing: 24) {
            Text("Rapture")
                .font(.largeTitle.weight(.semibold))

            Text(manager.state.greeting)
                .font(.title3)

            TextField("Enter your name", text: $nameInput)
                .textFieldStyle(.roundedBorder)
                .onSubmit {
                    manager.dispatch(.setName(name: nameInput))
                }

            Button("Greet") {
                manager.dispatch(.setName(name: nameInput))
            }
            .buttonStyle(.borderedProminent)
        }
        .padding(20)
    }
}
