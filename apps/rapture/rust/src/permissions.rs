use std::collections::BTreeSet;

use crate::control::GuildState;

pub const PERM_VIEW_CHANNEL: u64 = 1 << 0;
pub const PERM_SEND_MESSAGE: u64 = 1 << 1;
pub const PERM_MANAGE_MESSAGES: u64 = 1 << 2;
pub const PERM_MANAGE_CHANNELS: u64 = 1 << 3;
pub const PERM_MANAGE_ROLES: u64 = 1 << 4;
pub const PERM_KICK_MEMBERS: u64 = 1 << 5;
pub const PERM_BAN_MEMBERS: u64 = 1 << 6;
pub const PERM_ADMINISTRATOR: u64 = 1 << 7;
pub const PERM_CONNECT_VOICE: u64 = 1 << 8;
pub const PERM_SPEAK_VOICE: u64 = 1 << 9;
pub const PERM_MUTE_MEMBERS: u64 = 1 << 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ViewChannel,
    SendMessage,
    ManageMessages,
    ManageChannels,
    ManageRoles,
    KickMembers,
    BanMembers,
    Administrator,
    ConnectVoice,
    SpeakVoice,
    MuteMembers,
}

pub fn permission_bits(bits: &[u64]) -> u64 {
    bits.iter().copied().fold(0_u64, |acc, v| acc | v)
}

pub fn has_permission(
    guild: &GuildState,
    user: &str,
    channel_id: Option<&str>,
    permission: Permission,
) -> bool {
    if !guild.members.contains(user) {
        return false;
    }

    let role_ids: BTreeSet<String> = guild
        .member_roles
        .get(user)
        .cloned()
        .unwrap_or_else(BTreeSet::new);
    let mut base_mask = 0_u64;
    for role_id in &role_ids {
        if let Some(role) = guild.roles.get(role_id) {
            base_mask |= role.permissions;
        }
    }

    if (base_mask & PERM_ADMINISTRATOR) != 0 {
        return true;
    }

    if let Some(cid) = channel_id {
        if let Some(channel) = guild.channels.get(cid) {
            if channel.policy.deny_users.contains(user) {
                return false;
            }
            if channel.policy.allow_users.contains(user) {
                return true;
            }
            if role_ids
                .iter()
                .any(|r| channel.policy.deny_roles.contains(r))
            {
                return false;
            }
            if role_ids
                .iter()
                .any(|r| channel.policy.allow_roles.contains(r))
            {
                return true;
            }
        }
    }

    (base_mask & permission_bit(permission)) != 0
}

fn permission_bit(permission: Permission) -> u64 {
    match permission {
        Permission::ViewChannel => PERM_VIEW_CHANNEL,
        Permission::SendMessage => PERM_SEND_MESSAGE,
        Permission::ManageMessages => PERM_MANAGE_MESSAGES,
        Permission::ManageChannels => PERM_MANAGE_CHANNELS,
        Permission::ManageRoles => PERM_MANAGE_ROLES,
        Permission::KickMembers => PERM_KICK_MEMBERS,
        Permission::BanMembers => PERM_BAN_MEMBERS,
        Permission::Administrator => PERM_ADMINISTRATOR,
        Permission::ConnectVoice => PERM_CONNECT_VOICE,
        Permission::SpeakVoice => PERM_SPEAK_VOICE,
        Permission::MuteMembers => PERM_MUTE_MEMBERS,
    }
}
