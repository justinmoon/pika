import SwiftUI
import UIKit

struct NewGroupChatView: View {
    let state: NewGroupChatViewState
    let onCreateGroup: @MainActor (String, [String]) -> Void
    let onRefreshFollowList: @MainActor () -> Void
    @State private var groupName = ""
    @State private var selectedNpubs: [String] = []
    @State private var searchText = ""
    @State private var npubInput = ""
    @State private var showScanner = false
    @State private var showInvalidNpubAlert = false
    @State private var invalidNpubMessage = ""
    @State private var showManualEntrySheet = false

    private var filteredFollowList: [FollowListEntry] {
        let base = state.followList.filter { $0.npub != state.myNpub }
        guard !searchText.isEmpty else { return base }
        let query = searchText.lowercased()
        return base.filter { entry in
            if let name = entry.name, name.lowercased().contains(query) { return true }
            if let username = entry.username, username.lowercased().contains(query) { return true }
            if entry.npub.lowercased().contains(query) { return true }
            if entry.pubkey.lowercased().contains(query) { return true }
            return false
        }
    }

    private var canCreate: Bool {
        !groupName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !selectedNpubs.isEmpty
            && !state.isCreatingChat
    }

    var body: some View {
        let isLoading = state.isCreatingChat

        List {
            groupNameSection(isLoading: isLoading)
            if !selectedNpubs.isEmpty {
                selectedMembersSection(isLoading: isLoading)
            }
            quickActionsSection(isLoading: isLoading)
            followsSection(isLoading: isLoading)
            createButton(isLoading: isLoading)
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(Color(.systemGroupedBackground))
        .navigationTitle("New Group")
        .navigationBarTitleDisplayMode(.large)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    showManualEntrySheet = true
                } label: {
                    Image(systemName: "keyboard")
                }
                .disabled(isLoading)
                .accessibilityIdentifier(TestIds.newGroupManualEntry)
            }
        }
        .safeAreaInset(edge: .bottom) {
            NativeBottomSearchField(title: "Search follows", text: $searchText)
                .padding(.horizontal, 16)
                .padding(.top, 8)
                .padding(.bottom, 8)
                .background(.bar)
        }
        .onAppear {
            onRefreshFollowList()
        }
        .sheet(isPresented: $showScanner) {
            QrScannerSheet { scanned in
                handleIncomingPeer(scanned)
            }
        }
        .sheet(isPresented: $showManualEntrySheet, onDismiss: {
            npubInput = ""
        }) {
            manualEntrySheet(isLoading: isLoading)
        }
        .alert("Invalid code", isPresented: $showInvalidNpubAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(invalidNpubMessage)
        }
    }

    private func groupNameSection(isLoading: Bool) -> some View {
        Section {
            TextField("Group name", text: $groupName)
                .disabled(isLoading)
                .accessibilityIdentifier(TestIds.newGroupName)
        }
    }

    private func selectedMembersSection(isLoading: Bool) -> some View {
        Section("Members (\(selectedNpubs.count))") {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(selectedNpubs, id: \.self) { npub in
                        selectedChip(npub: npub, isLoading: isLoading)
                    }
                }
                .padding(.vertical, 4)
            }
        }
    }

    private func quickActionsSection(isLoading: Bool) -> some View {
        Section {
            HStack(spacing: 8) {
                NativeQuickActionButton(
                    title: "Paste Code",
                    systemImage: "doc.on.clipboard",
                    isPrimary: true,
                    accessibilityIdentifier: TestIds.newGroupPasteCode
                ) {
                    handlePaste()
                }
                .disabled(isLoading)

                if ProcessInfo.processInfo.isiOSAppOnMac == false {
                    NativeQuickActionButton(
                        title: "Scan Code",
                        systemImage: "qrcode.viewfinder",
                        accessibilityIdentifier: TestIds.newGroupScanQr
                    ) {
                        showScanner = true
                    }
                    .disabled(isLoading)
                }
            }
            .padding(.vertical, 8)
        }
    }

    @ViewBuilder
    private func followsSection(isLoading: Bool) -> some View {
        Section {
            if state.isFetchingFollowList && state.followList.isEmpty {
                HStack {
                    Spacer()
                    ProgressView("Loading follows...")
                    Spacer()
                }
            } else if state.followList.isEmpty {
                emptyStateRow(
                    title: "No follows found",
                    message: "Follow people or paste a code to add them."
                )
            } else if filteredFollowList.isEmpty {
                emptyStateRow(
                    title: "No matches found",
                    message: "Try a different search."
                )
            } else {
                ForEach(filteredFollowList, id: \.pubkey) { entry in
                    Button {
                        toggleSelection(npub: entry.npub)
                    } label: {
                        followListRow(entry: entry)
                            .padding(.vertical, 8)
                    }
                    .buttonStyle(.plain)
                    .disabled(isLoading)
                }
            }
        } header: {
            HStack(spacing: 6) {
                Text("Follows")
                if state.isFetchingFollowList {
                    ProgressView()
                        .controlSize(.small)
                }
            }
        }
    }

    private func createButton(isLoading: Bool) -> some View {
        Section {
            Button {
                onCreateGroup(
                    groupName.trimmingCharacters(in: .whitespacesAndNewlines),
                    selectedNpubs
                )
            } label: {
                HStack {
                    Spacer()
                    if isLoading {
                        HStack(spacing: 8) {
                            ProgressView().tint(.white)
                            Text("Creating...")
                        }
                    } else {
                        Text("Create Group")
                    }
                    Spacer()
                }
                .frame(height: 50)
            }
            .buttonStyle(.borderedProminent)
            .disabled(!canCreate)
            .accessibilityIdentifier(TestIds.newGroupCreate)
        }
    }

    private func followListRow(entry: FollowListEntry) -> some View {
        let isSelected = selectedNpubs.contains(entry.npub)
        return HStack(spacing: 12) {
            AvatarView(
                name: entry.name,
                npub: entry.npub,
                pictureUrl: entry.pictureUrl,
                size: 40
            )

            VStack(alignment: .leading, spacing: 2) {
                if let name = entry.name {
                    Text(name)
                        .font(.body)
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                }
                Text(truncatedNpub(entry.npub))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer()

            if isSelected {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.tint)
            }
        }
        .contentShape(Rectangle())
    }

    private func selectedChip(npub: String, isLoading: Bool) -> some View {
        let entry = state.followList.first { $0.npub == npub }
        let displayName = entry?.name ?? truncatedNpub(npub)
        return HStack(spacing: 4) {
            Text(displayName)
                .font(.caption)
                .foregroundStyle(.primary)
                .lineLimit(1)
            Button {
                selectedNpubs.removeAll { $0 == npub }
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .disabled(isLoading)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(Color(.secondarySystemFill), in: Capsule())
    }

    private func toggleSelection(npub: String) {
        if let idx = selectedNpubs.firstIndex(of: npub) {
            selectedNpubs.remove(at: idx)
        } else {
            selectedNpubs.append(npub)
        }
    }

    private func handlePaste() {
        let raw = UIPasteboard.general.string ?? ""
        handleIncomingPeer(raw)
    }

    private func emptyStateRow(title: String, message: String) -> some View {
        VStack(spacing: 6) {
            Text(title)
                .font(.subheadline.weight(.semibold))
            Text(message)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 20)
    }

    @discardableResult
    private func handleIncomingPeer(_ input: String) -> Bool {
        let normalized = normalizePeerKey(input: input)
        guard isValidPeerKey(input: normalized) else {
            invalidNpubMessage = "Enter, paste, or scan a valid code (npub1… or 64-character hex public key)."
            showInvalidNpubAlert = true
            return false
        }
        if !selectedNpubs.contains(normalized) {
            selectedNpubs.append(normalized)
            return true
        }
        invalidNpubMessage = "That person is already selected."
        showInvalidNpubAlert = true
        return false
    }

    private func manualEntrySheet(isLoading: Bool) -> some View {
        NavigationStack {
            Form {
                Section {
                    Text("Enter a code to add someone to the group.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)

                    TextField("Code", text: $npubInput)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .accessibilityIdentifier(TestIds.newGroupPeerNpub)

                    Button("Add Member") {
                        let peer = normalizePeerKey(input: npubInput)
                        if handleIncomingPeer(peer) {
                            npubInput = ""
                            showManualEntrySheet = false
                        }
                    }
                    .disabled(normalizePeerKey(input: npubInput).isEmpty || isLoading)
                    .accessibilityIdentifier(TestIds.newGroupAddMember)
                }
            }
            .navigationTitle("Add Member")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        npubInput = ""
                        showManualEntrySheet = false
                    }
                }
            }
        }
    }

    private func truncatedNpub(_ npub: String) -> String {
        if npub.count <= 20 { return npub }
        return String(npub.prefix(12)) + "..." + String(npub.suffix(4))
    }
}

#if DEBUG
#Preview("New Group - Loading") {
    NavigationStack {
        NewGroupChatView(
            state: NewGroupChatViewState(
                isCreatingChat: false,
                isFetchingFollowList: true,
                followList: [],
                myNpub: nil
            ),
            onCreateGroup: { _, _ in },
            onRefreshFollowList: {}
        )
    }
}

#Preview("New Group - Populated") {
    NavigationStack {
        NewGroupChatView(
            state: NewGroupChatViewState(
                isCreatingChat: false,
                isFetchingFollowList: false,
                followList: PreviewAppState.sampleFollowList,
                myNpub: nil
            ),
            onCreateGroup: { _, _ in },
            onRefreshFollowList: {}
        )
    }
}

#Preview("New Group - Creating") {
    NavigationStack {
        NewGroupChatView(
            state: NewGroupChatViewState(
                isCreatingChat: true,
                isFetchingFollowList: false,
                followList: PreviewAppState.sampleFollowList,
                myNpub: nil
            ),
            onCreateGroup: { _, _ in },
            onRefreshFollowList: {}
        )
    }
}
#endif
