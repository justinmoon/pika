import SwiftUI

struct ContentView: View {
    @Bindable var manager: AppManager
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass

    @State private var actor = "alice"
    @State private var draft = ""
    @State private var editingMessageId: String?
    @State private var editingText = ""

    @State private var showGuildSheet = false
    @State private var showChannelSheet = false
    @State private var showAdminSheet = false

    private var selectedGuild: GuildSummary? {
        guard let selectedGuildId = manager.state.selectedGuildId else { return nil }
        return manager.state.guilds.first(where: { $0.guildId == selectedGuildId })
    }

    private var selectedChannel: ChannelSummary? {
        guard
            let guild = selectedGuild,
            let selectedChannelId = manager.state.selectedChannelId
        else { return nil }
        return guild.channels.first(where: { $0.channelId == selectedChannelId })
    }

    private var isCompact: Bool {
        horizontalSizeClass == .compact
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let toast = manager.state.toast, !toast.isEmpty {
                    Text(toast)
                        .font(.footnote)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(Color.red.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
                        .padding(.horizontal, 12)
                        .padding(.top, 8)
                }

                HStack(spacing: 0) {
                    GuildRail(
                        guilds: manager.state.guilds,
                        selectedGuildId: manager.state.selectedGuildId,
                        onSelect: { guildId in
                            manager.dispatch(.selectGuild(guildId: guildId))
                        },
                        onManage: { showGuildSheet = true }
                    )

                    if !isCompact {
                        ChannelSidebar(
                            guild: selectedGuild,
                            selectedChannelId: manager.state.selectedChannelId,
                            onSelect: { channelId in
                                guard let guildId = selectedGuild?.guildId else { return }
                                manager.dispatch(.selectChannel(guildId: guildId, channelId: channelId))
                            },
                            onManage: { showChannelSheet = true }
                        )
                    }

                    Divider()

                    TimelinePane(
                        actor: $actor,
                        guild: selectedGuild,
                        channel: selectedChannel,
                        timeline: manager.state.timeline,
                        voiceRoom: manager.state.voiceRoom,
                        draft: $draft,
                        editingMessageId: $editingMessageId,
                        editingText: $editingText,
                        onSend: sendMessage,
                        onSaveEdit: saveEdit,
                        onCancelEdit: {
                            editingMessageId = nil
                            editingText = ""
                        },
                        onStartEdit: { message in
                            editingMessageId = message.messageId
                            editingText = message.content
                        },
                        onDelete: deleteMessage,
                        onToggleThumbsUp: toggleThumbsUp,
                        onJoinVoice: joinVoice,
                        onLeaveVoice: leaveVoice,
                        onSetVoiceMuted: setVoiceMuted,
                        onSetVoiceSpeaking: setVoiceSpeaking
                    )
                }
            }
            .navigationTitle(channelTitle)
            .toolbar {
                ToolbarItemGroup(placement: .topBarLeading) {
                    Button("Servers") { showGuildSheet = true }
                    Button("Channels") { showChannelSheet = true }
                        .disabled(selectedGuild == nil)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Admin") { showAdminSheet = true }
                }
            }
        }
        .onAppear(perform: ensureSelection)
        .onChange(of: manager.state.rev) { _, _ in
            ensureSelection()
        }
        .sheet(isPresented: $showGuildSheet) {
            GuildManagementSheet(
                guilds: manager.state.guilds,
                selectedGuildId: manager.state.selectedGuildId,
                actor: $actor,
                onSelectGuild: { guildId in
                    manager.dispatch(.selectGuild(guildId: guildId))
                    showGuildSheet = false
                },
                onCreateGuild: { guildId, guildName in
                    manager.dispatch(
                        .createGuild(
                            guildId: guildId,
                            name: guildName,
                            actorPubkey: actor.trimmed()
                        )
                    )
                }
            )
            .presentationDetents([.large])
        }
        .sheet(isPresented: $showChannelSheet) {
            ChannelManagementSheet(
                guild: selectedGuild,
                selectedChannelId: manager.state.selectedChannelId,
                onSelectChannel: { channelId in
                    guard let guildId = selectedGuild?.guildId else { return }
                    manager.dispatch(.selectChannel(guildId: guildId, channelId: channelId))
                    showChannelSheet = false
                },
                onCreateChannel: { guildId, channelId, channelName, kind in
                    manager.dispatch(
                        .createChannel(
                            guildId: guildId,
                            channelId: channelId,
                            name: channelName,
                            kind: kind,
                            actorPubkey: actor.trimmed()
                        )
                    )
                }
            )
            .presentationDetents([.large])
        }
        .sheet(isPresented: $showAdminSheet) {
            AdminSheet(
                actor: $actor,
                defaultGuildId: manager.state.selectedGuildId ?? "g-1",
                defaultChannelId: manager.state.selectedChannelId ?? "c-general",
                onDispatch: { action in manager.dispatch(action) }
            )
            .presentationDetents([.large])
        }
    }

    private var channelTitle: String {
        if let channel = selectedChannel {
            return "#\(channel.name)"
        }
        if let guild = selectedGuild {
            return guild.name
        }
        return "Rapture"
    }

    private func ensureSelection() {
        guard !manager.state.guilds.isEmpty else { return }

        if manager.state.selectedGuildId == nil {
            manager.dispatch(.selectGuild(guildId: manager.state.guilds[0].guildId))
            return
        }

        guard
            let guild = selectedGuild,
            !guild.channels.isEmpty,
            manager.state.selectedChannelId == nil
        else { return }

        manager.dispatch(.selectChannel(guildId: guild.guildId, channelId: guild.channels[0].channelId))
    }

    private func sendMessage() {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel
        else { return }

        let text = draft.trimmed()
        guard !text.isEmpty else { return }

        manager.dispatch(
            .sendMessage(
                guildId: guild.guildId,
                channelId: channel.channelId,
                actorPubkey: actor.trimmed(),
                content: text
            )
        )
        draft = ""
    }

    private func joinVoice() {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel
        else { return }

        manager.dispatch(
            .joinVoice(
                guildId: guild.guildId,
                channelId: channel.channelId,
                actorPubkey: actor.trimmed()
            )
        )
    }

    private func leaveVoice() {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel
        else { return }

        manager.dispatch(
            .leaveVoice(
                guildId: guild.guildId,
                channelId: channel.channelId,
                actorPubkey: actor.trimmed()
            )
        )
    }

    private func setVoiceMuted(_ muted: Bool) {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel
        else { return }

        manager.dispatch(
            .setVoiceMuted(
                guildId: guild.guildId,
                channelId: channel.channelId,
                actorPubkey: actor.trimmed(),
                muted: muted
            )
        )
    }

    private func setVoiceSpeaking(_ speaking: Bool) {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel
        else { return }

        manager.dispatch(
            .setVoiceSpeaking(
                guildId: guild.guildId,
                channelId: channel.channelId,
                actorPubkey: actor.trimmed(),
                speaking: speaking
            )
        )
    }

    private func saveEdit() {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel,
            let messageId = editingMessageId
        else { return }

        let text = editingText.trimmed()
        guard !text.isEmpty else { return }

        manager.dispatch(
            .editMessage(
                guildId: guild.guildId,
                channelId: channel.channelId,
                actorPubkey: actor.trimmed(),
                messageId: messageId,
                content: text
            )
        )
        editingMessageId = nil
        editingText = ""
    }

    private func deleteMessage(_ messageId: String) {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel
        else { return }

        manager.dispatch(
            .deleteMessage(
                guildId: guild.guildId,
                channelId: channel.channelId,
                actorPubkey: actor.trimmed(),
                messageId: messageId
            )
        )

        if editingMessageId == messageId {
            editingMessageId = nil
            editingText = ""
        }
    }

    private func toggleThumbsUp(_ message: TimelineMessageSummary) {
        guard
            let guild = selectedGuild,
            let channel = selectedChannel
        else { return }

        let actorValue = actor.trimmed()
        let hasReacted = message.reactions
            .first(where: { $0.emoji == ":+1:" })?
            .actors
            .contains(actorValue)
            ?? false

        if hasReacted {
            manager.dispatch(
                .removeReaction(
                    guildId: guild.guildId,
                    channelId: channel.channelId,
                    actorPubkey: actorValue,
                    messageId: message.messageId,
                    emoji: ":+1:"
                )
            )
        } else {
            manager.dispatch(
                .putReaction(
                    guildId: guild.guildId,
                    channelId: channel.channelId,
                    actorPubkey: actorValue,
                    messageId: message.messageId,
                    emoji: ":+1:"
                )
            )
        }
    }
}

private struct GuildRail: View {
    let guilds: [GuildSummary]
    let selectedGuildId: String?
    let onSelect: (String) -> Void
    let onManage: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text("S")
                .font(.caption.weight(.bold))
                .foregroundStyle(.secondary)

            ScrollView {
                VStack(spacing: 10) {
                    ForEach(guilds, id: \.guildId) { guild in
                        let selected = guild.guildId == selectedGuildId
                        Button {
                            onSelect(guild.guildId)
                        } label: {
                            Text(guild.name.prefix(1).uppercased())
                                .font(.headline)
                                .frame(width: 46, height: 46)
                                .foregroundStyle(selected ? Color.white : .primary)
                                .background(selected ? Color.accentColor : Color.secondary.opacity(0.18), in: Circle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.vertical, 4)
            }

            Button("+") {
                onManage()
            }
            .buttonStyle(.bordered)
        }
        .frame(width: 74)
        .padding(.vertical, 8)
        .background(Color.secondary.opacity(0.08))
    }
}

private struct ChannelSidebar: View {
    let guild: GuildSummary?
    let selectedChannelId: String?
    let onSelect: (String) -> Void
    let onManage: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(guild?.name ?? "No server")
                    .font(.headline)
                    .lineLimit(1)
                Spacer()
                Button("+") { onManage() }
                    .buttonStyle(.bordered)
                    .disabled(guild == nil)
            }

            Divider()

            if let guild {
                if guild.channels.isEmpty {
                    Text("No channels")
                        .foregroundStyle(.secondary)
                } else {
                    ScrollView {
                        VStack(spacing: 6) {
                            ForEach(guild.channels, id: \.channelId) { channel in
                                let selected = channel.channelId == selectedChannelId
                                Button {
                                    onSelect(channel.channelId)
                                } label: {
                                    HStack {
                                        Text("#\(channel.name)")
                                            .lineLimit(1)
                                        Spacer()
                                        Text(kindLabel(channel.kind))
                                            .font(.caption)
                                            .foregroundStyle(.secondary)
                                    }
                                    .padding(.horizontal, 10)
                                    .padding(.vertical, 8)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .background(selected ? Color.secondary.opacity(0.2) : Color.clear, in: RoundedRectangle(cornerRadius: 8))
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
            } else {
                Text("Select a server")
                    .foregroundStyle(.secondary)
            }

            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(width: 230)
        .background(Color.secondary.opacity(0.08))
    }
}

private struct TimelinePane: View {
    @Binding var actor: String
    let guild: GuildSummary?
    let channel: ChannelSummary?
    let timeline: [TimelineMessageSummary]
    let voiceRoom: VoiceRoomSummary?

    @Binding var draft: String
    @Binding var editingMessageId: String?
    @Binding var editingText: String

    let onSend: () -> Void
    let onSaveEdit: () -> Void
    let onCancelEdit: () -> Void
    let onStartEdit: (TimelineMessageSummary) -> Void
    let onDelete: (String) -> Void
    let onToggleThumbsUp: (TimelineMessageSummary) -> Void
    let onJoinVoice: () -> Void
    let onLeaveVoice: () -> Void
    let onSetVoiceMuted: (Bool) -> Void
    let onSetVoiceSpeaking: (Bool) -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                TextField("Actor", text: $actor)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .textFieldStyle(.roundedBorder)
                Text(guild?.name ?? "No server")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                Text(channel.map { "#\($0.name)" } ?? "No channel")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
            }
            .padding(10)

            Divider()

            if guild == nil {
                EmptyStateView(message: "Create or select a server to begin.")
            } else if channel == nil {
                EmptyStateView(message: "Create or select a channel in \(guild!.name).")
            } else {
                let isVoiceChannel = channel!.kind == .voice
                let selfVoice = voiceRoom?.participants.first(where: { $0.pubkey == actor.trimmed() })

                if isVoiceChannel {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Voice controls")
                            .font(.headline)
                        Text(voiceRoom?.activeSessionId.map { "session: \($0)" } ?? "session: not started")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Text("participants: \(voiceRoom?.participants.count ?? 0) • tracks: \(voiceRoom?.trackCount ?? 0)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        if let participants = voiceRoom?.participants, !participants.isEmpty {
                            Text(
                                participants.map { p in
                                    let flags = [
                                        p.muted ? "muted" : nil,
                                        p.speaking ? "speaking" : nil,
                                    ].compactMap { $0 }.joined(separator: "/")
                                    return flags.isEmpty ? p.pubkey : "\(p.pubkey) (\(flags))"
                                }.joined(separator: ", ")
                            )
                            .font(.caption)
                        }
                        HStack {
                            Button("Join") { onJoinVoice() }
                                .buttonStyle(.borderedProminent)
                            Button("Leave") { onLeaveVoice() }
                                .buttonStyle(.bordered)
                        }
                        HStack {
                            Button((selfVoice?.muted ?? false) ? "Unmute" : "Mute") {
                                onSetVoiceMuted(!(selfVoice?.muted ?? false))
                            }
                            .buttonStyle(.bordered)
                            Button((selfVoice?.speaking ?? false) ? "Stop Speaking" : "Speak") {
                                onSetVoiceSpeaking(!(selfVoice?.speaking ?? false))
                            }
                            .buttonStyle(.bordered)
                        }
                    }
                    .padding(10)
                    .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
                    .padding(.horizontal, 10)
                    .padding(.top, 8)
                }

                List {
                    if timeline.isEmpty {
                        Text("No messages yet in #\(channel!.name).")
                            .foregroundStyle(.secondary)
                    }

                    ForEach(timeline, id: \.messageId) { message in
                        TimelineRow(
                            message: message,
                            actor: actor.trimmed(),
                            onStartEdit: { onStartEdit(message) },
                            onDelete: { onDelete(message.messageId) },
                            onToggleThumbsUp: { onToggleThumbsUp(message) }
                        )
                        .listRowInsets(EdgeInsets(top: 6, leading: 10, bottom: 6, trailing: 10))
                        .listRowSeparator(.hidden)
                    }
                }
                .listStyle(.plain)

                if !isVoiceChannel && editingMessageId != nil {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Editing message")
                            .font(.headline)
                        TextField("Updated content", text: $editingText)
                            .textFieldStyle(.roundedBorder)
                        HStack {
                            Button("Save") { onSaveEdit() }
                                .buttonStyle(.borderedProminent)
                            Button("Cancel") { onCancelEdit() }
                                .buttonStyle(.bordered)
                        }
                    }
                    .padding(10)
                    .background(Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
                    .padding(.horizontal, 10)
                    .padding(.bottom, 8)
                }

                if !isVoiceChannel {
                    HStack(spacing: 8) {
                        TextField("Message #\(channel!.name)", text: $draft)
                            .textFieldStyle(.roundedBorder)
                        Button("Send") { onSend() }
                            .buttonStyle(.borderedProminent)
                    }
                    .padding(10)
                }
            }
        }
    }
}

private struct TimelineRow: View {
    let message: TimelineMessageSummary
    let actor: String
    let onStartEdit: () -> Void
    let onDelete: () -> Void
    let onToggleThumbsUp: () -> Void

    private var mine: Bool { !actor.isEmpty && actor == message.author }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(message.author)
                    .font(.subheadline.weight(.semibold))
                Spacer()
                if message.edited {
                    Text("edited")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            Text(message.content)
                .font(.body)

            if !message.reactions.isEmpty {
                HStack(spacing: 8) {
                    ForEach(message.reactions, id: \.emoji) { reaction in
                        let count = reaction.actors.count
                        let me = reaction.actors.contains(actor)
                        Text("\(reaction.emoji) \(count)")
                            .font(.caption)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            .background(me ? Color.accentColor.opacity(0.2) : Color.secondary.opacity(0.12), in: Capsule())
                    }
                }
            }

            HStack(spacing: 8) {
                Button("+1") { onToggleThumbsUp() }
                Button("Edit") { onStartEdit() }
                Button("Delete") { onDelete() }
            }
            .buttonStyle(.bordered)
            .font(.caption)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(mine ? Color.accentColor.opacity(0.12) : Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
    }
}

private struct GuildManagementSheet: View {
    let guilds: [GuildSummary]
    let selectedGuildId: String?
    @Binding var actor: String
    let onSelectGuild: (String) -> Void
    let onCreateGuild: (String, String) -> Void

    @State private var guildId = "g-1"
    @State private var guildName = "Guild One"

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("Actor pubkey", text: $actor)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)

                    ForEach(guilds, id: \.guildId) { guild in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(guild.name).font(.headline)
                            Text("\(guild.channels.count) channels")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            Button(guild.guildId == selectedGuildId ? "Selected" : "Open") {
                                onSelectGuild(guild.guildId)
                            }
                            .buttonStyle(.bordered)
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 10))
                    }

                    Divider()

                    Text("Create server")
                        .font(.headline)
                    TextField("Guild ID", text: $guildId)
                        .textFieldStyle(.roundedBorder)
                    TextField("Guild name", text: $guildName)
                        .textFieldStyle(.roundedBorder)
                    Button("Create") {
                        onCreateGuild(guildId.trimmed(), guildName.trimmed())
                    }
                    .buttonStyle(.borderedProminent)
                }
                .padding(16)
            }
            .navigationTitle("Servers")
        }
    }
}

private struct ChannelManagementSheet: View {
    let guild: GuildSummary?
    let selectedChannelId: String?
    let onSelectChannel: (String) -> Void
    let onCreateChannel: (String, String, String, ChannelKind) -> Void

    @State private var channelId = "c-general"
    @State private var channelName = "general"
    @State private var channelKind: ChannelKind = .text

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let guild {
                        ForEach(guild.channels, id: \.channelId) { channel in
                            VStack(alignment: .leading, spacing: 6) {
                                Text("#\(channel.name)").font(.headline)
                                Text(kindLabel(channel.kind))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                Button(channel.channelId == selectedChannelId ? "Selected" : "Open") {
                                    onSelectChannel(channel.channelId)
                                }
                                .buttonStyle(.bordered)
                            }
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 10))
                        }

                        Divider()

                        Text("Create channel")
                            .font(.headline)
                        TextField("Channel ID", text: $channelId)
                            .textFieldStyle(.roundedBorder)
                        TextField("Channel name", text: $channelName)
                            .textFieldStyle(.roundedBorder)

                        Picker("Kind", selection: $channelKind) {
                            Text("text").tag(ChannelKind.text)
                            Text("voice").tag(ChannelKind.voice)
                            Text("private").tag(ChannelKind.`private`)
                            Text("thread").tag(ChannelKind.thread)
                        }
                        .pickerStyle(.segmented)

                        Button("Create") {
                            onCreateChannel(
                                guild.guildId,
                                channelId.trimmed(),
                                channelName.trimmed(),
                                channelKind
                            )
                        }
                        .buttonStyle(.borderedProminent)
                    } else {
                        Text("Select a server first")
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(16)
            }
            .navigationTitle("Channels")
        }
    }
}

private struct AdminSheet: View {
    @Binding var actor: String
    let defaultGuildId: String
    let defaultChannelId: String
    let onDispatch: (AppAction) -> Void

    @State private var memberGuildId = ""
    @State private var memberPubkey = "bob"

    @State private var roleGuildId = ""
    @State private var roleMemberPubkey = "bob"
    @State private var roleIdsCsv = "role-everyone"

    @State private var policyGuildId = ""
    @State private var policyChannelId = ""
    @State private var allowRolesCsv = ""
    @State private var denyRolesCsv = ""
    @State private var allowUsersCsv = "bob"
    @State private var denyUsersCsv = ""

    @State private var removeGuildId = ""
    @State private var removeChannelId = ""
    @State private var removeMemberPubkey = "bob"

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("Actor pubkey", text: $actor)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textFieldStyle(.roundedBorder)

                    SectionCard("Member actions") {
                        TextField("Guild ID", text: $memberGuildId)
                            .textFieldStyle(.roundedBorder)
                        TextField("Member pubkey", text: $memberPubkey)
                            .textFieldStyle(.roundedBorder)
                        HStack {
                            Button("Invite") {
                                onDispatch(
                                    .inviteMember(
                                        guildId: memberGuildId.trimmed(),
                                        memberPubkey: memberPubkey.trimmed(),
                                        actorPubkey: actor.trimmed()
                                    )
                                )
                            }
                            Button("Kick") {
                                onDispatch(
                                    .kickMember(
                                        guildId: memberGuildId.trimmed(),
                                        memberPubkey: memberPubkey.trimmed(),
                                        actorPubkey: actor.trimmed()
                                    )
                                )
                            }
                            Button("Ban") {
                                onDispatch(
                                    .banMember(
                                        guildId: memberGuildId.trimmed(),
                                        memberPubkey: memberPubkey.trimmed(),
                                        actorPubkey: actor.trimmed()
                                    )
                                )
                            }
                        }
                        .buttonStyle(.bordered)
                    }

                    SectionCard("Set member roles") {
                        TextField("Guild ID", text: $roleGuildId)
                            .textFieldStyle(.roundedBorder)
                        TextField("Member pubkey", text: $roleMemberPubkey)
                            .textFieldStyle(.roundedBorder)
                        TextField("Role IDs (csv)", text: $roleIdsCsv)
                            .textFieldStyle(.roundedBorder)
                        Button("Apply") {
                            onDispatch(
                                .setMemberRoles(
                                    guildId: roleGuildId.trimmed(),
                                    memberPubkey: roleMemberPubkey.trimmed(),
                                    roleIds: parseCsv(roleIdsCsv),
                                    actorPubkey: actor.trimmed()
                                )
                            )
                        }
                        .buttonStyle(.bordered)
                    }

                    SectionCard("Set channel permissions") {
                        TextField("Guild ID", text: $policyGuildId)
                            .textFieldStyle(.roundedBorder)
                        TextField("Channel ID", text: $policyChannelId)
                            .textFieldStyle(.roundedBorder)
                        TextField("Allow roles (csv)", text: $allowRolesCsv)
                            .textFieldStyle(.roundedBorder)
                        TextField("Deny roles (csv)", text: $denyRolesCsv)
                            .textFieldStyle(.roundedBorder)
                        TextField("Allow users (csv)", text: $allowUsersCsv)
                            .textFieldStyle(.roundedBorder)
                        TextField("Deny users (csv)", text: $denyUsersCsv)
                            .textFieldStyle(.roundedBorder)
                        Button("Apply") {
                            onDispatch(
                                .setChannelPermissions(
                                    guildId: policyGuildId.trimmed(),
                                    channelId: policyChannelId.trimmed(),
                                    allowRoles: parseCsv(allowRolesCsv),
                                    denyRoles: parseCsv(denyRolesCsv),
                                    allowUsers: parseCsv(allowUsersCsv),
                                    denyUsers: parseCsv(denyUsersCsv),
                                    actorPubkey: actor.trimmed()
                                )
                            )
                        }
                        .buttonStyle(.bordered)
                    }

                    SectionCard("Remove member from channel") {
                        TextField("Guild ID", text: $removeGuildId)
                            .textFieldStyle(.roundedBorder)
                        TextField("Channel ID", text: $removeChannelId)
                            .textFieldStyle(.roundedBorder)
                        TextField("Member pubkey", text: $removeMemberPubkey)
                            .textFieldStyle(.roundedBorder)
                        Button("Remove") {
                            onDispatch(
                                .removeMemberFromChannel(
                                    guildId: removeGuildId.trimmed(),
                                    channelId: removeChannelId.trimmed(),
                                    memberPubkey: removeMemberPubkey.trimmed(),
                                    actorPubkey: actor.trimmed()
                                )
                            )
                        }
                        .buttonStyle(.bordered)
                    }
                }
                .padding(16)
            }
            .navigationTitle("Admin")
        }
        .onAppear {
            if memberGuildId.isEmpty { memberGuildId = defaultGuildId }
            if roleGuildId.isEmpty { roleGuildId = defaultGuildId }
            if policyGuildId.isEmpty { policyGuildId = defaultGuildId }
            if policyChannelId.isEmpty { policyChannelId = defaultChannelId }
            if removeGuildId.isEmpty { removeGuildId = defaultGuildId }
            if removeChannelId.isEmpty { removeChannelId = defaultChannelId }
        }
    }
}

private struct SectionCard<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.headline)
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 10))
    }
}

private struct EmptyStateView: View {
    let message: String

    var body: some View {
        VStack {
            Spacer()
            Text(message)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .padding(.horizontal, 20)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private func parseCsv(_ value: String) -> [String] {
    value
        .split(separator: ",")
        .map { String($0).trimmed() }
        .filter { !$0.isEmpty }
}

private func kindLabel(_ kind: ChannelKind) -> String {
    switch kind {
    case .text: return "text"
    case .voice: return "voice"
    case .private: return "private"
    case .thread: return "thread"
    }
}

private extension String {
    func trimmed() -> String {
        trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
