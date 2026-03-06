import SwiftUI
import UIKit

struct LoginView: View {
    let state: LoginViewState
    let onCreateAccount: @MainActor () -> Void
    let onLogin: @MainActor (String) -> Void
    let onBunkerLogin: @MainActor (String) -> Void
    let onNostrConnectLogin: @MainActor () -> Void
    let onResetNostrConnectPairing: @MainActor () -> Void
    @State private var nsecInput = ""
    @State private var bunkerUriInput = ""
    @State private var showAdvanced = false

    private var trimmedNsecInput: String {
        nsecInput.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var trimmedBunkerUriInput: String {
        bunkerUriInput.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        let createBusy = state.creatingAccount
        let loginBusy = state.loggingIn
        let anyBusy = createBusy || loginBusy

        List {
            Section {
                VStack(spacing: 0) {
                    Image("PikaLogo")
                        .resizable()
                        .scaledToFit()
                        .frame(width: 140, height: 140)
                        .clipShape(RoundedRectangle(cornerRadius: 28))

                    Text("Pika")
                        .font(.largeTitle.weight(.bold))
                        .padding(.top, 16)

                    Text("Encrypted messaging over Nostr")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .padding(.top, 4)
                }
                .frame(maxWidth: .infinity)
                .padding(.vertical, 24)
            }
            .listRowBackground(Color.clear)

            Section {
                Button {
                    onCreateAccount()
                } label: {
                    if createBusy {
                        ProgressView()
                            .tint(.white)
                            .frame(maxWidth: .infinity)
                    } else {
                        Text("Create Account")
                            .frame(maxWidth: .infinity)
                    }
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .disabled(anyBusy)
                .accessibilityIdentifier(TestIds.loginCreateAccount)
            } footer: {
                Text("Or sign in with your account private key.")
            }

            Section("Account Private Key") {
                HStack(spacing: 12) {
                    SecureField("Enter your private key (nsec123...)", text: $nsecInput)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textContentType(.password)
                        .disabled(anyBusy)
                        .accessibilityIdentifier(TestIds.loginNsecInput)

                    Button("Paste") {
                        guard let pasted = UIPasteboard.general.string?
                            .trimmingCharacters(in: .whitespacesAndNewlines),
                              !pasted.isEmpty else { return }
                        nsecInput = pasted
                    }
                    .accessibilityIdentifier(TestIds.loginPastePrivateKey)
                    .disabled(anyBusy)
                }

                Button {
                    onLogin(trimmedNsecInput)
                } label: {
                    if loginBusy {
                        ProgressView()
                            .frame(maxWidth: .infinity)
                    } else {
                        Text("Log In")
                            .frame(maxWidth: .infinity)
                    }
                }
                .buttonStyle(.bordered)
                .disabled(anyBusy || trimmedNsecInput.isEmpty)
                .accessibilityIdentifier(TestIds.loginSubmit)
            }

            Section {
                DisclosureGroup("Advanced", isExpanded: $showAdvanced) {
                    TextField("Enter bunker URI", text: $bunkerUriInput)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .disabled(anyBusy)
                        .accessibilityIdentifier(TestIds.loginBunkerUriInput)

                    Button {
                        onBunkerLogin(trimmedBunkerUriInput)
                    } label: {
                        if loginBusy {
                            ProgressView()
                                .frame(maxWidth: .infinity)
                        } else {
                            Text("Log In with Bunker")
                                .frame(maxWidth: .infinity)
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                    .disabled(anyBusy || trimmedBunkerUriInput.isEmpty)
                    .accessibilityIdentifier(TestIds.loginBunkerSubmit)

                    Button {
                        onNostrConnectLogin()
                    } label: {
                        if loginBusy {
                            ProgressView()
                                .frame(maxWidth: .infinity)
                        } else {
                            Text("Log In with Nostr Connect")
                                .frame(maxWidth: .infinity)
                        }
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .disabled(anyBusy)
                    .accessibilityIdentifier(TestIds.loginNostrConnectSubmit)

                    Button("Reset Nostr Connect Pairing") {
                        onResetNostrConnectPairing()
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.regular)
                    .disabled(anyBusy)
                    .accessibilityIdentifier(TestIds.loginNostrConnectReset)
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(Color(.systemGroupedBackground))
    }
}

#if DEBUG
#Preview("Login") {
    LoginView(
        state: LoginViewState(creatingAccount: false, loggingIn: false),
        onCreateAccount: {},
        onLogin: { _ in },
        onBunkerLogin: { _ in },
        onNostrConnectLogin: {},
        onResetNostrConnectPairing: {}
    )
}

#Preview("Login - Busy") {
    LoginView(
        state: LoginViewState(creatingAccount: false, loggingIn: true),
        onCreateAccount: {},
        onLogin: { _ in },
        onBunkerLogin: { _ in },
        onNostrConnectLogin: {},
        onResetNostrConnectPairing: {}
    )
}

#Preview("Login - Creating") {
    LoginView(
        state: LoginViewState(creatingAccount: true, loggingIn: false),
        onCreateAccount: {},
        onLogin: { _ in },
        onBunkerLogin: { _ in },
        onNostrConnectLogin: {},
        onResetNostrConnectPairing: {}
    )
}
#endif
