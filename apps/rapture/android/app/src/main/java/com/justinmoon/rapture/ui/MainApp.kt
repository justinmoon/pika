package com.justinmoon.rapture.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.justinmoon.rapture.AppManager
import com.justinmoon.rapture.rust.AppAction
import com.justinmoon.rapture.rust.ChannelKind
import com.justinmoon.rapture.rust.ChannelSummary
import com.justinmoon.rapture.rust.GuildSummary
import com.justinmoon.rapture.rust.TimelineMessageSummary
import com.justinmoon.rapture.rust.VoiceRoomSummary

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainApp(manager: AppManager) {
    val state = manager.state

    var actor by remember { mutableStateOf("alice") }
    var draft by remember { mutableStateOf("") }
    var editingMessageId by remember { mutableStateOf<String?>(null) }
    var editingText by remember { mutableStateOf("") }

    var showGuilds by remember { mutableStateOf(false) }
    var showChannels by remember { mutableStateOf(false) }
    var showAdmin by remember { mutableStateOf(false) }

    LaunchedEffect(state.rev, state.guilds, state.selectedGuildId, state.selectedChannelId) {
        if (state.guilds.isEmpty()) {
            return@LaunchedEffect
        }

        if (state.selectedGuildId == null) {
            manager.dispatch(AppAction.SelectGuild(guildId = state.guilds.first().guildId))
            return@LaunchedEffect
        }

        val selectedGuild = state.guilds.firstOrNull { it.guildId == state.selectedGuildId }
        if (selectedGuild != null && selectedGuild.channels.isNotEmpty() && state.selectedChannelId == null) {
            manager.dispatch(
                AppAction.SelectChannel(
                    guildId = selectedGuild.guildId,
                    channelId = selectedGuild.channels.first().channelId,
                )
            )
        }
    }

    val selectedGuild = state.guilds.firstOrNull { it.guildId == state.selectedGuildId }
    val selectedChannel = selectedGuild?.channels?.firstOrNull { it.channelId == state.selectedChannelId }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        val heading = when {
                            selectedChannel != null -> "#${selectedChannel.name}"
                            selectedGuild != null -> selectedGuild.name
                            else -> "Rapture"
                        }
                        Text(heading)
                        Text(
                            "rev ${state.rev}",
                            style = MaterialTheme.typography.labelSmall,
                        )
                    }
                },
                navigationIcon = {
                    TextButton(onClick = { showGuilds = true }) {
                        Text("Servers")
                    }
                },
                actions = {
                    TextButton(onClick = { showChannels = true }, enabled = selectedGuild != null) {
                        Text("Channels")
                    }
                    TextButton(onClick = { showAdmin = true }) {
                        Text("Admin")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            if (!state.toast.isNullOrBlank()) {
                Text(
                    state.toast ?: "",
                    color = MaterialTheme.colorScheme.onErrorContainer,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp)
                        .background(
                            MaterialTheme.colorScheme.errorContainer,
                            shape = MaterialTheme.shapes.small,
                        )
                        .padding(10.dp),
                )
            }

            BoxWithConstraints(modifier = Modifier.fillMaxSize()) {
                val wideLayout = maxWidth >= 760.dp
                Row(modifier = Modifier.fillMaxSize()) {
                    GuildRail(
                        guilds = state.guilds,
                        selectedGuildId = state.selectedGuildId,
                        onSelectGuild = { guildId ->
                            manager.dispatch(AppAction.SelectGuild(guildId = guildId))
                        },
                        onOpenGuilds = { showGuilds = true },
                    )

                    if (wideLayout) {
                        ChannelPane(
                            guild = selectedGuild,
                            selectedChannelId = state.selectedChannelId,
                            onSelectChannel = { channelId ->
                                val guildId = selectedGuild?.guildId ?: return@ChannelPane
                                manager.dispatch(
                                    AppAction.SelectChannel(
                                        guildId = guildId,
                                        channelId = channelId,
                                    )
                                )
                            },
                            onOpenChannels = { showChannels = true },
                        )
                    }

                    TimelinePane(
                        modifier = Modifier.weight(1f),
                        actor = actor,
                        onActorChange = { actor = it },
                        guild = selectedGuild,
                        channel = selectedChannel,
                        timeline = state.timeline,
                        voiceRoom = state.voiceRoom,
                        draft = draft,
                        onDraftChange = { draft = it },
                        editingMessageId = editingMessageId,
                        editingText = editingText,
                        onEditTextChange = { editingText = it },
                        onStartEdit = { message ->
                            editingMessageId = message.messageId
                            editingText = message.content
                        },
                        onCancelEdit = {
                            editingMessageId = null
                            editingText = ""
                        },
                        onSend = {
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            val text = draft.trim()
                            if (text.isEmpty()) return@TimelinePane

                            manager.dispatch(
                                AppAction.SendMessage(
                                    guildId = guildId,
                                    channelId = channelId,
                                    actorPubkey = actor.trim(),
                                    content = text,
                                )
                            )
                            draft = ""
                        },
                        onSaveEdit = {
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            val messageId = editingMessageId ?: return@TimelinePane
                            val text = editingText.trim()
                            if (text.isEmpty()) return@TimelinePane

                            manager.dispatch(
                                AppAction.EditMessage(
                                    guildId = guildId,
                                    channelId = channelId,
                                    actorPubkey = actor.trim(),
                                    messageId = messageId,
                                    content = text,
                                )
                            )
                            editingMessageId = null
                            editingText = ""
                        },
                        onDelete = { messageId ->
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            manager.dispatch(
                                AppAction.DeleteMessage(
                                    guildId = guildId,
                                    channelId = channelId,
                                    actorPubkey = actor.trim(),
                                    messageId = messageId,
                                )
                            )
                            if (editingMessageId == messageId) {
                                editingMessageId = null
                                editingText = ""
                            }
                        },
                        onToggleThumbsUp = { message ->
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            val actorTrimmed = actor.trim()
                            val hasReacted = message.reactions
                                .firstOrNull { it.emoji == ":+1:" }
                                ?.actors
                                ?.contains(actorTrimmed)
                                ?: false

                            if (hasReacted) {
                                manager.dispatch(
                                    AppAction.RemoveReaction(
                                        guildId = guildId,
                                        channelId = channelId,
                                        actorPubkey = actorTrimmed,
                                        messageId = message.messageId,
                                        emoji = ":+1:",
                                    )
                                )
                            } else {
                                manager.dispatch(
                                    AppAction.PutReaction(
                                        guildId = guildId,
                                        channelId = channelId,
                                        actorPubkey = actorTrimmed,
                                        messageId = message.messageId,
                                        emoji = ":+1:",
                                    )
                                )
                            }
                        },
                        onJoinVoice = {
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            manager.dispatch(
                                AppAction.JoinVoice(
                                    guildId = guildId,
                                    channelId = channelId,
                                    actorPubkey = actor.trim(),
                                )
                            )
                        },
                        onLeaveVoice = {
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            manager.dispatch(
                                AppAction.LeaveVoice(
                                    guildId = guildId,
                                    channelId = channelId,
                                    actorPubkey = actor.trim(),
                                )
                            )
                        },
                        onSetVoiceMuted = { muted ->
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            manager.dispatch(
                                AppAction.SetVoiceMuted(
                                    guildId = guildId,
                                    channelId = channelId,
                                    actorPubkey = actor.trim(),
                                    muted = muted,
                                )
                            )
                        },
                        onSetVoiceSpeaking = { speaking ->
                            val guildId = selectedGuild?.guildId ?: return@TimelinePane
                            val channelId = selectedChannel?.channelId ?: return@TimelinePane
                            manager.dispatch(
                                AppAction.SetVoiceSpeaking(
                                    guildId = guildId,
                                    channelId = channelId,
                                    actorPubkey = actor.trim(),
                                    speaking = speaking,
                                )
                            )
                        },
                    )
                }
            }
        }
    }

    if (showGuilds) {
        GuildSheet(
            guilds = state.guilds,
            selectedGuildId = state.selectedGuildId,
            actor = actor,
            onActorChange = { actor = it },
            onClose = { showGuilds = false },
            onSelectGuild = {
                manager.dispatch(AppAction.SelectGuild(guildId = it))
                showGuilds = false
            },
            onCreateGuild = { guildId, guildName ->
                manager.dispatch(
                    AppAction.CreateGuild(
                        guildId = guildId,
                        name = guildName,
                        actorPubkey = actor.trim(),
                    )
                )
            },
        )
    }

    if (showChannels) {
        ChannelSheet(
            guild = selectedGuild,
            selectedChannelId = state.selectedChannelId,
            onClose = { showChannels = false },
            onSelectChannel = {
                val guildId = selectedGuild?.guildId ?: return@ChannelSheet
                manager.dispatch(AppAction.SelectChannel(guildId = guildId, channelId = it))
                showChannels = false
            },
            onCreateChannel = { guildId, channelId, channelName, kind ->
                manager.dispatch(
                    AppAction.CreateChannel(
                        guildId = guildId,
                        channelId = channelId,
                        name = channelName,
                        kind = kind,
                        actorPubkey = actor.trim(),
                    )
                )
            },
        )
    }

    if (showAdmin) {
        AdminSheet(
            actor = actor,
            onActorChange = { actor = it },
            defaultGuildId = state.selectedGuildId ?: "g-1",
            defaultChannelId = state.selectedChannelId ?: "c-general",
            onClose = { showAdmin = false },
            onAction = { manager.dispatch(it) },
        )
    }
}

@Composable
private fun GuildRail(
    guilds: List<GuildSummary>,
    selectedGuildId: String?,
    onSelectGuild: (String) -> Unit,
    onOpenGuilds: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxHeight()
            .width(76.dp)
            .background(MaterialTheme.colorScheme.surfaceContainerLow)
            .padding(vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("S", style = MaterialTheme.typography.labelMedium)
        LazyColumn(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            contentPadding = PaddingValues(vertical = 4.dp),
        ) {
            items(guilds, key = { it.guildId }) { guild ->
                val selected = guild.guildId == selectedGuildId
                val bg = if (selected) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.secondaryContainer
                }
                TextButton(
                    onClick = { onSelectGuild(guild.guildId) },
                    modifier = Modifier
                        .size(52.dp)
                        .background(bg, CircleShape),
                ) {
                    Text(
                        guild.name.take(1).uppercase(),
                        color = if (selected) {
                            MaterialTheme.colorScheme.onPrimary
                        } else {
                            MaterialTheme.colorScheme.onSecondaryContainer
                        },
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
        TextButton(onClick = onOpenGuilds) {
            Text("+")
        }
    }
}

@Composable
private fun ChannelPane(
    guild: GuildSummary?,
    selectedChannelId: String?,
    onSelectChannel: (String) -> Unit,
    onOpenChannels: () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxHeight()
            .width(220.dp)
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .padding(horizontal = 10.dp, vertical = 8.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                guild?.name ?: "No server",
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
            )
            TextButton(onClick = onOpenChannels, enabled = guild != null) {
                Text("+")
            }
        }
        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))

        if (guild == null || guild.channels.isEmpty()) {
            Text(
                "No channels",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodyMedium,
            )
            return
        }

        LazyColumn(
            verticalArrangement = Arrangement.spacedBy(4.dp),
            contentPadding = PaddingValues(bottom = 8.dp),
        ) {
            items(guild.channels, key = { it.channelId }) { channel ->
                val selected = channel.channelId == selectedChannelId
                val rowBg = if (selected) {
                    MaterialTheme.colorScheme.secondaryContainer
                } else {
                    MaterialTheme.colorScheme.surfaceContainer
                }
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(rowBg, shape = MaterialTheme.shapes.small)
                        .padding(horizontal = 8.dp, vertical = 6.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text("#${channel.name}", maxLines = 1)
                    TextButton(onClick = { onSelectChannel(channel.channelId) }) {
                        Text(if (selected) "Open" else "Go")
                    }
                }
            }
        }
    }
}

@Composable
private fun TimelinePane(
    modifier: Modifier,
    actor: String,
    onActorChange: (String) -> Unit,
    guild: GuildSummary?,
    channel: ChannelSummary?,
    timeline: List<TimelineMessageSummary>,
    voiceRoom: VoiceRoomSummary?,
    draft: String,
    onDraftChange: (String) -> Unit,
    editingMessageId: String?,
    editingText: String,
    onEditTextChange: (String) -> Unit,
    onStartEdit: (TimelineMessageSummary) -> Unit,
    onCancelEdit: () -> Unit,
    onSend: () -> Unit,
    onSaveEdit: () -> Unit,
    onDelete: (String) -> Unit,
    onToggleThumbsUp: (TimelineMessageSummary) -> Unit,
    onJoinVoice: () -> Unit,
    onLeaveVoice: () -> Unit,
    onSetVoiceMuted: (Boolean) -> Unit,
    onSetVoiceSpeaking: (Boolean) -> Unit,
) {
    Column(modifier = modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            OutlinedTextField(
                value = actor,
                onValueChange = onActorChange,
                label = { Text("Actor") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            Text(
                guild?.name ?: "No server",
                style = MaterialTheme.typography.labelLarge,
            )
            Text(
                channel?.let { "#${it.name}" } ?: "No channel",
                style = MaterialTheme.typography.labelLarge,
            )
        }
        HorizontalDivider()

        if (guild == null) {
            EmptyState(
                message = "Create or select a server to begin.",
                modifier = Modifier.weight(1f),
            )
        } else if (channel == null) {
            EmptyState(
                message = "Create or select a channel in ${guild.name}.",
                modifier = Modifier.weight(1f),
            )
        } else {
            val isVoiceChannel = channel.kind == ChannelKind.VOICE
            val selfVoiceState = voiceRoom
                ?.participants
                ?.firstOrNull { it.pubkey == actor.trim() }

            if (isVoiceChannel) {
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Column(
                        modifier = Modifier.padding(10.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text("Voice controls", fontWeight = FontWeight.SemiBold)
                        Text(
                            voiceRoom?.activeSessionId?.let { "session: $it" } ?: "session: not started",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(
                            "participants: ${voiceRoom?.participants?.size ?: 0} • tracks: ${voiceRoom?.trackCount ?: 0u}",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (!voiceRoom?.participants.isNullOrEmpty()) {
                            Text(
                                voiceRoom!!.participants.joinToString(", ") { p ->
                                    val flags = mutableListOf<String>()
                                    if (p.muted) flags += "muted"
                                    if (p.speaking) flags += "speaking"
                                    if (flags.isEmpty()) p.pubkey else "${p.pubkey} (${flags.joinToString("/")})"
                                },
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = onJoinVoice) { Text("Join") }
                            Button(onClick = onLeaveVoice) { Text("Leave") }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = { onSetVoiceMuted(!(selfVoiceState?.muted ?: false)) }) {
                                Text(if ((selfVoiceState?.muted ?: false)) "Unmute" else "Mute")
                            }
                            Button(onClick = { onSetVoiceSpeaking(!(selfVoiceState?.speaking ?: false)) }) {
                                Text(if ((selfVoiceState?.speaking ?: false)) "Stop Speaking" else "Speak")
                            }
                        }
                    }
                }
            }

            LazyColumn(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(8.dp),
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 10.dp),
            ) {
                items(timeline, key = { it.messageId }) { message ->
                    TimelineMessageCard(
                        message = message,
                        actor = actor.trim(),
                        onStartEdit = { onStartEdit(message) },
                        onDelete = { onDelete(message.messageId) },
                        onToggleThumbsUp = { onToggleThumbsUp(message) },
                    )
                }

                if (timeline.isEmpty()) {
                    item {
                        Text(
                            "No messages yet in #${channel.name}.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(top = 12.dp),
                        )
                    }
                }
            }

            if (!isVoiceChannel && editingMessageId != null) {
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp),
                ) {
                    Column(
                        modifier = Modifier.padding(10.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text("Editing message", fontWeight = FontWeight.SemiBold)
                        OutlinedTextField(
                            value = editingText,
                            onValueChange = onEditTextChange,
                            label = { Text("Updated content") },
                            modifier = Modifier.fillMaxWidth(),
                        )
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = onSaveEdit) { Text("Save") }
                            TextButton(onClick = onCancelEdit) { Text("Cancel") }
                        }
                    }
                }
                Spacer(Modifier.height(8.dp))
            }

            if (!isVoiceChannel) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedTextField(
                        value = draft,
                        onValueChange = onDraftChange,
                        label = { Text("Message #${channel.name}") },
                        modifier = Modifier.weight(1f),
                    )
                    Button(onClick = onSend) {
                        Text("Send")
                    }
                }
            }
        }
    }
}

@Composable
private fun TimelineMessageCard(
    message: TimelineMessageSummary,
    actor: String,
    onStartEdit: () -> Unit,
    onDelete: () -> Unit,
    onToggleThumbsUp: () -> Unit,
) {
    val mine = actor.isNotBlank() && message.author == actor
    val bg = if (mine) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh

    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(bg)
                .padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(message.author, fontWeight = FontWeight.SemiBold)
                Text(
                    if (message.edited) "edited" else "",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Text(message.content)

            if (message.reactions.isNotEmpty()) {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    message.reactions.forEach { reaction ->
                        val count = reaction.actors.size
                        val me = reaction.actors.contains(actor)
                        val label = if (count > 0) "${reaction.emoji} $count" else reaction.emoji
                        FilterChip(
                            selected = me,
                            onClick = onToggleThumbsUp,
                            label = { Text(label) },
                        )
                    }
                }
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(onClick = onToggleThumbsUp) {
                    Text("+1")
                }
                TextButton(onClick = onStartEdit) {
                    Text("Edit")
                }
                TextButton(onClick = onDelete) {
                    Text("Delete")
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun GuildSheet(
    guilds: List<GuildSummary>,
    selectedGuildId: String?,
    actor: String,
    onActorChange: (String) -> Unit,
    onClose: () -> Unit,
    onSelectGuild: (String) -> Unit,
    onCreateGuild: (String, String) -> Unit,
) {
    var guildId by remember { mutableStateOf("g-1") }
    var guildName by remember { mutableStateOf("Guild One") }

    ModalBottomSheet(onDismissRequest = onClose) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text("Servers", style = MaterialTheme.typography.titleLarge)
            OutlinedTextField(
                value = actor,
                onValueChange = onActorChange,
                label = { Text("Actor pubkey") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            guilds.forEach { guild ->
                val selected = guild.guildId == selectedGuildId
                Card(modifier = Modifier.fillMaxWidth()) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(10.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column {
                            Text(guild.name, fontWeight = FontWeight.SemiBold)
                            Text("${guild.channels.size} channels", style = MaterialTheme.typography.labelSmall)
                        }
                        TextButton(onClick = { onSelectGuild(guild.guildId) }) {
                            Text(if (selected) "Selected" else "Open")
                        }
                    }
                }
            }

            HorizontalDivider()
            Text("Create server", style = MaterialTheme.typography.titleMedium)
            OutlinedTextField(
                value = guildId,
                onValueChange = { guildId = it },
                label = { Text("Guild ID") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = guildName,
                onValueChange = { guildName = it },
                label = { Text("Guild name") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Button(
                onClick = { onCreateGuild(guildId.trim(), guildName.trim()) },
                modifier = Modifier.align(Alignment.End),
            ) {
                Text("Create")
            }

            Spacer(Modifier.size(12.dp))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ChannelSheet(
    guild: GuildSummary?,
    selectedChannelId: String?,
    onClose: () -> Unit,
    onSelectChannel: (String) -> Unit,
    onCreateChannel: (String, String, String, ChannelKind) -> Unit,
) {
    var channelId by remember { mutableStateOf("c-general") }
    var channelName by remember { mutableStateOf("general") }
    var kind by remember { mutableStateOf(ChannelKind.TEXT) }

    ModalBottomSheet(onDismissRequest = onClose) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text("Channels", style = MaterialTheme.typography.titleLarge)
            if (guild == null) {
                Text("Select a server first.")
            } else {
                guild.channels.forEach { ch ->
                    val selected = ch.channelId == selectedChannelId
                    Card(modifier = Modifier.fillMaxWidth()) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(10.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column {
                                Text("#${ch.name}", fontWeight = FontWeight.SemiBold)
                                Text(kindLabel(ch.kind), style = MaterialTheme.typography.labelSmall)
                            }
                            TextButton(onClick = { onSelectChannel(ch.channelId) }) {
                                Text(if (selected) "Selected" else "Open")
                            }
                        }
                    }
                }

                HorizontalDivider()
                Text("Create channel", style = MaterialTheme.typography.titleMedium)
                OutlinedTextField(
                    value = channelId,
                    onValueChange = { channelId = it },
                    label = { Text("Channel ID") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = channelName,
                    onValueChange = { channelName = it },
                    label = { Text("Channel name") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    ChannelKind.values().forEach { option ->
                        FilterChip(
                            selected = kind == option,
                            onClick = { kind = option },
                            label = { Text(option.name.lowercase()) },
                        )
                    }
                }
                Button(
                    onClick = {
                        onCreateChannel(
                            guild.guildId,
                            channelId.trim(),
                            channelName.trim(),
                            kind,
                        )
                    },
                    modifier = Modifier.align(Alignment.End),
                ) {
                    Text("Create")
                }
            }

            Spacer(Modifier.size(12.dp))
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AdminSheet(
    actor: String,
    onActorChange: (String) -> Unit,
    defaultGuildId: String,
    defaultChannelId: String,
    onClose: () -> Unit,
    onAction: (AppAction) -> Unit,
) {
    var memberGuildId by remember { mutableStateOf(defaultGuildId) }
    var memberPubkey by remember { mutableStateOf("bob") }

    var roleGuildId by remember { mutableStateOf(defaultGuildId) }
    var roleMemberPubkey by remember { mutableStateOf("bob") }
    var roleIdsCsv by remember { mutableStateOf("role-everyone") }

    var policyGuildId by remember { mutableStateOf(defaultGuildId) }
    var policyChannelId by remember { mutableStateOf(defaultChannelId) }
    var allowRolesCsv by remember { mutableStateOf("") }
    var denyRolesCsv by remember { mutableStateOf("") }
    var allowUsersCsv by remember { mutableStateOf("bob") }
    var denyUsersCsv by remember { mutableStateOf("") }

    var removeGuildId by remember { mutableStateOf(defaultGuildId) }
    var removeChannelId by remember { mutableStateOf(defaultChannelId) }
    var removeMemberPubkey by remember { mutableStateOf("bob") }

    ModalBottomSheet(onDismissRequest = onClose) {
        LazyColumn(
            modifier = Modifier.fillMaxWidth(),
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item { Text("Admin", style = MaterialTheme.typography.titleLarge) }
            item {
                OutlinedTextField(
                    value = actor,
                    onValueChange = onActorChange,
                    label = { Text("Actor pubkey") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            item {
                AdminCard("Member actions") {
                    OutlinedTextField(
                        value = memberGuildId,
                        onValueChange = { memberGuildId = it },
                        label = { Text("Guild ID") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = memberPubkey,
                        onValueChange = { memberPubkey = it },
                        label = { Text("Member pubkey") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = {
                            onAction(
                                AppAction.InviteMember(
                                    guildId = memberGuildId.trim(),
                                    memberPubkey = memberPubkey.trim(),
                                    actorPubkey = actor.trim(),
                                )
                            )
                        }) { Text("Invite") }
                        Button(onClick = {
                            onAction(
                                AppAction.KickMember(
                                    guildId = memberGuildId.trim(),
                                    memberPubkey = memberPubkey.trim(),
                                    actorPubkey = actor.trim(),
                                )
                            )
                        }) { Text("Kick") }
                        Button(onClick = {
                            onAction(
                                AppAction.BanMember(
                                    guildId = memberGuildId.trim(),
                                    memberPubkey = memberPubkey.trim(),
                                    actorPubkey = actor.trim(),
                                )
                            )
                        }) { Text("Ban") }
                    }
                }
            }

            item {
                AdminCard("Set member roles") {
                    OutlinedTextField(
                        value = roleGuildId,
                        onValueChange = { roleGuildId = it },
                        label = { Text("Guild ID") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = roleMemberPubkey,
                        onValueChange = { roleMemberPubkey = it },
                        label = { Text("Member pubkey") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = roleIdsCsv,
                        onValueChange = { roleIdsCsv = it },
                        label = { Text("Role IDs (csv)") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Button(onClick = {
                        onAction(
                            AppAction.SetMemberRoles(
                                guildId = roleGuildId.trim(),
                                memberPubkey = roleMemberPubkey.trim(),
                                roleIds = parseCsv(roleIdsCsv),
                                actorPubkey = actor.trim(),
                            )
                        )
                    }) { Text("Apply") }
                }
            }

            item {
                AdminCard("Set channel permissions") {
                    OutlinedTextField(
                        value = policyGuildId,
                        onValueChange = { policyGuildId = it },
                        label = { Text("Guild ID") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = policyChannelId,
                        onValueChange = { policyChannelId = it },
                        label = { Text("Channel ID") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = allowRolesCsv,
                        onValueChange = { allowRolesCsv = it },
                        label = { Text("Allow roles (csv)") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = denyRolesCsv,
                        onValueChange = { denyRolesCsv = it },
                        label = { Text("Deny roles (csv)") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = allowUsersCsv,
                        onValueChange = { allowUsersCsv = it },
                        label = { Text("Allow users (csv)") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = denyUsersCsv,
                        onValueChange = { denyUsersCsv = it },
                        label = { Text("Deny users (csv)") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Button(onClick = {
                        onAction(
                            AppAction.SetChannelPermissions(
                                guildId = policyGuildId.trim(),
                                channelId = policyChannelId.trim(),
                                allowRoles = parseCsv(allowRolesCsv),
                                denyRoles = parseCsv(denyRolesCsv),
                                allowUsers = parseCsv(allowUsersCsv),
                                denyUsers = parseCsv(denyUsersCsv),
                                actorPubkey = actor.trim(),
                            )
                        )
                    }) { Text("Apply") }
                }
            }

            item {
                AdminCard("Remove member from channel") {
                    OutlinedTextField(
                        value = removeGuildId,
                        onValueChange = { removeGuildId = it },
                        label = { Text("Guild ID") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = removeChannelId,
                        onValueChange = { removeChannelId = it },
                        label = { Text("Channel ID") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = removeMemberPubkey,
                        onValueChange = { removeMemberPubkey = it },
                        label = { Text("Member pubkey") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Button(onClick = {
                        onAction(
                            AppAction.RemoveMemberFromChannel(
                                guildId = removeGuildId.trim(),
                                channelId = removeChannelId.trim(),
                                memberPubkey = removeMemberPubkey.trim(),
                                actorPubkey = actor.trim(),
                            )
                        )
                    }) { Text("Remove") }
                }
            }

            item { Spacer(Modifier.size(12.dp)) }
        }
    }
}

@Composable
private fun AdminCard(
    title: String,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, fontWeight = FontWeight.SemiBold)
            content()
        }
    }
}

@Composable
private fun EmptyState(message: String, modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .fillMaxSize()
            .padding(20.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = message,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}

private fun parseCsv(value: String): List<String> =
    value
        .split(',')
        .map { it.trim() }
        .filter { it.isNotEmpty() }

private fun kindLabel(kind: ChannelKind): String = when (kind) {
    ChannelKind.TEXT -> "text"
    ChannelKind.VOICE -> "voice"
    ChannelKind.PRIVATE -> "private"
    ChannelKind.THREAD -> "thread"
}
