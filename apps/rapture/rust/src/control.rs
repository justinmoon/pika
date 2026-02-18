use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::permissions::{
    has_permission, permission_bits, Permission, PERM_ADMINISTRATOR, PERM_BAN_MEMBERS,
    PERM_CONNECT_VOICE, PERM_KICK_MEMBERS, PERM_MANAGE_CHANNELS, PERM_MANAGE_MESSAGES,
    PERM_MANAGE_ROLES, PERM_MUTE_MEMBERS, PERM_SEND_MESSAGE, PERM_SPEAK_VOICE, PERM_VIEW_CHANNEL,
};
use crate::ChannelKind;

pub const CONTROL_SCHEMA_V1: &str = "rapture.control.v1";
const ROLE_OWNER: &str = "role-owner";
const ROLE_EVERYONE: &str = "role-everyone";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlState {
    pub guilds: BTreeMap<String, GuildState>,
    pub seen_op_ids: BTreeSet<String>,
}

impl Default for ControlState {
    fn default() -> Self {
        Self {
            guilds: BTreeMap::new(),
            seen_op_ids: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuildState {
    pub guild_id: String,
    pub name: String,
    pub created_by: String,
    pub members: BTreeSet<String>,
    pub roles: BTreeMap<String, RoleState>,
    pub member_roles: BTreeMap<String, BTreeSet<String>>,
    pub channels: BTreeMap<String, ChannelState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoleState {
    pub role_id: String,
    pub name: String,
    pub permissions: u64,
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelState {
    pub channel_id: String,
    pub guild_id: String,
    pub name: String,
    pub kind: ChannelKind,
    pub policy: ChannelPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChannelPolicy {
    pub allow_roles: BTreeSet<String>,
    pub deny_roles: BTreeSet<String>,
    pub allow_users: BTreeSet<String>,
    pub deny_users: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlEnvelope {
    pub schema: String,
    pub guild_id: String,
    pub actor: String,
    pub op_id: String,
    pub ts_ms: i64,
    #[serde(flatten)]
    pub body: ControlBody,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", content = "body")]
pub enum ControlBody {
    #[serde(rename = "guild.create")]
    GuildCreate { name: String },
    #[serde(rename = "role.upsert")]
    RoleUpsert {
        role_id: String,
        name: String,
        permissions: u64,
        priority: i32,
    },
    #[serde(rename = "member.add")]
    MemberAdd { member_pubkey: String },
    #[serde(rename = "member.remove")]
    MemberRemove { member_pubkey: String },
    #[serde(rename = "member.roles.set")]
    MemberRolesSet {
        member_pubkey: String,
        role_ids: Vec<String>,
    },
    #[serde(rename = "channel.create")]
    ChannelCreate {
        channel_id: String,
        name: String,
        kind: ChannelKind,
    },
    #[serde(rename = "channel.permissions.set")]
    ChannelPermissionsSet {
        channel_id: String,
        allow_roles: Vec<String>,
        deny_roles: Vec<String>,
        allow_users: Vec<String>,
        deny_users: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    Duplicate,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("unsupported schema: {0}")]
    UnknownSchema(String),
    #[error("guild already exists: {0}")]
    GuildExists(String),
    #[error("guild not found: {0}")]
    GuildNotFound(String),
    #[error("channel not found: {guild_id}/{channel_id}")]
    ChannelNotFound {
        guild_id: String,
        channel_id: String,
    },
    #[error("role not found: {guild_id}/{role_id}")]
    RoleNotFound { guild_id: String, role_id: String },
    #[error("member not found: {guild_id}/{member}")]
    MemberNotFound { guild_id: String, member: String },
    #[error("permission denied: actor {actor} lacks {permission}")]
    PermissionDenied { actor: String, permission: String },
}

impl ControlEnvelope {
    pub fn guild_create(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        actor: String,
        name: String,
    ) -> Self {
        Self {
            schema: CONTROL_SCHEMA_V1.to_string(),
            guild_id,
            actor,
            op_id,
            ts_ms,
            body: ControlBody::GuildCreate { name },
        }
    }

    pub fn role_upsert(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        actor: String,
        role_id: String,
        name: String,
        permissions: u64,
        priority: i32,
    ) -> Self {
        Self {
            schema: CONTROL_SCHEMA_V1.to_string(),
            guild_id,
            actor,
            op_id,
            ts_ms,
            body: ControlBody::RoleUpsert {
                role_id,
                name,
                permissions,
                priority,
            },
        }
    }

    pub fn member_add(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        actor: String,
        member_pubkey: String,
    ) -> Self {
        Self {
            schema: CONTROL_SCHEMA_V1.to_string(),
            guild_id,
            actor,
            op_id,
            ts_ms,
            body: ControlBody::MemberAdd { member_pubkey },
        }
    }

    pub fn member_remove(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        actor: String,
        member_pubkey: String,
    ) -> Self {
        Self {
            schema: CONTROL_SCHEMA_V1.to_string(),
            guild_id,
            actor,
            op_id,
            ts_ms,
            body: ControlBody::MemberRemove { member_pubkey },
        }
    }

    pub fn member_roles_set(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        actor: String,
        member_pubkey: String,
        role_ids: Vec<String>,
    ) -> Self {
        Self {
            schema: CONTROL_SCHEMA_V1.to_string(),
            guild_id,
            actor,
            op_id,
            ts_ms,
            body: ControlBody::MemberRolesSet {
                member_pubkey,
                role_ids,
            },
        }
    }

    pub fn channel_create(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        actor: String,
        channel_id: String,
        name: String,
        kind: ChannelKind,
    ) -> Self {
        Self {
            schema: CONTROL_SCHEMA_V1.to_string(),
            guild_id,
            actor,
            op_id,
            ts_ms,
            body: ControlBody::ChannelCreate {
                channel_id,
                name,
                kind,
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn channel_permissions_set(
        op_id: String,
        ts_ms: i64,
        guild_id: String,
        actor: String,
        channel_id: String,
        allow_roles: Vec<String>,
        deny_roles: Vec<String>,
        allow_users: Vec<String>,
        deny_users: Vec<String>,
    ) -> Self {
        Self {
            schema: CONTROL_SCHEMA_V1.to_string(),
            guild_id,
            actor,
            op_id,
            ts_ms,
            body: ControlBody::ChannelPermissionsSet {
                channel_id,
                allow_roles,
                deny_roles,
                allow_users,
                deny_users,
            },
        }
    }
}

impl ControlState {
    pub fn apply(&mut self, envelope: ControlEnvelope) -> Result<ApplyOutcome, ControlError> {
        if envelope.schema != CONTROL_SCHEMA_V1 {
            return Err(ControlError::UnknownSchema(envelope.schema));
        }

        if self.seen_op_ids.contains(&envelope.op_id) {
            return Ok(ApplyOutcome::Duplicate);
        }

        match envelope.body {
            ControlBody::GuildCreate { name } => {
                if self.guilds.contains_key(&envelope.guild_id) {
                    return Err(ControlError::GuildExists(envelope.guild_id));
                }
                self.guilds.insert(
                    envelope.guild_id.clone(),
                    new_guild(envelope.guild_id.clone(), name, envelope.actor.clone()),
                );
            }
            ControlBody::RoleUpsert {
                role_id,
                name,
                permissions,
                priority,
            } => {
                let guild = self.guild_mut(&envelope.guild_id)?;
                require_permission(guild, &envelope.actor, None, Permission::ManageRoles)?;
                guild.roles.insert(
                    role_id.clone(),
                    RoleState {
                        role_id,
                        name,
                        permissions,
                        priority,
                    },
                );
            }
            ControlBody::MemberAdd { member_pubkey } => {
                let guild = self.guild_mut(&envelope.guild_id)?;
                require_permission(guild, &envelope.actor, None, Permission::ManageRoles)?;
                guild.members.insert(member_pubkey.clone());
                let roles = guild
                    .member_roles
                    .entry(member_pubkey)
                    .or_insert_with(BTreeSet::new);
                roles.insert(ROLE_EVERYONE.to_string());
            }
            ControlBody::MemberRemove { member_pubkey } => {
                let guild = self.guild_mut(&envelope.guild_id)?;
                require_permission(guild, &envelope.actor, None, Permission::KickMembers)?;
                if !guild.members.contains(&member_pubkey) {
                    return Err(ControlError::MemberNotFound {
                        guild_id: envelope.guild_id,
                        member: member_pubkey,
                    });
                }
                guild.members.remove(&member_pubkey);
                guild.member_roles.remove(&member_pubkey);
            }
            ControlBody::MemberRolesSet {
                member_pubkey,
                role_ids,
            } => {
                let guild = self.guild_mut(&envelope.guild_id)?;
                require_permission(guild, &envelope.actor, None, Permission::ManageRoles)?;
                if !guild.members.contains(&member_pubkey) {
                    return Err(ControlError::MemberNotFound {
                        guild_id: envelope.guild_id,
                        member: member_pubkey,
                    });
                }

                for role_id in &role_ids {
                    if !guild.roles.contains_key(role_id) {
                        return Err(ControlError::RoleNotFound {
                            guild_id: envelope.guild_id,
                            role_id: role_id.clone(),
                        });
                    }
                }

                let mut set: BTreeSet<String> = role_ids.into_iter().collect();
                set.insert(ROLE_EVERYONE.to_string());
                guild.member_roles.insert(member_pubkey, set);
            }
            ControlBody::ChannelCreate {
                channel_id,
                name,
                kind,
            } => {
                let guild = self.guild_mut(&envelope.guild_id)?;
                require_permission(guild, &envelope.actor, None, Permission::ManageChannels)?;
                guild.channels.insert(
                    channel_id.clone(),
                    ChannelState {
                        channel_id,
                        guild_id: envelope.guild_id.clone(),
                        name,
                        kind,
                        policy: ChannelPolicy::default(),
                    },
                );
            }
            ControlBody::ChannelPermissionsSet {
                channel_id,
                allow_roles,
                deny_roles,
                allow_users,
                deny_users,
            } => {
                let guild = self.guild_mut(&envelope.guild_id)?;
                require_permission(guild, &envelope.actor, None, Permission::ManageChannels)?;
                let channel = guild.channels.get_mut(&channel_id).ok_or_else(|| {
                    ControlError::ChannelNotFound {
                        guild_id: envelope.guild_id.clone(),
                        channel_id: channel_id.clone(),
                    }
                })?;
                channel.policy = ChannelPolicy {
                    allow_roles: allow_roles.into_iter().collect(),
                    deny_roles: deny_roles.into_iter().collect(),
                    allow_users: allow_users.into_iter().collect(),
                    deny_users: deny_users.into_iter().collect(),
                };
            }
        }

        self.seen_op_ids.insert(envelope.op_id);
        Ok(ApplyOutcome::Applied)
    }

    fn guild_mut(&mut self, guild_id: &str) -> Result<&mut GuildState, ControlError> {
        self.guilds
            .get_mut(guild_id)
            .ok_or_else(|| ControlError::GuildNotFound(guild_id.to_string()))
    }
}

fn require_permission(
    guild: &GuildState,
    actor: &str,
    channel_id: Option<&str>,
    permission: Permission,
) -> Result<(), ControlError> {
    if has_permission(guild, actor, channel_id, permission) {
        return Ok(());
    }

    Err(ControlError::PermissionDenied {
        actor: actor.to_string(),
        permission: format!("{permission:?}"),
    })
}

fn new_guild(guild_id: String, name: String, creator: String) -> GuildState {
    let mut members = BTreeSet::new();
    members.insert(creator.clone());

    let mut roles = BTreeMap::new();
    roles.insert(
        ROLE_OWNER.to_string(),
        RoleState {
            role_id: ROLE_OWNER.to_string(),
            name: "Owner".to_string(),
            permissions: permission_bits(&[
                PERM_VIEW_CHANNEL,
                PERM_SEND_MESSAGE,
                PERM_MANAGE_MESSAGES,
                PERM_MANAGE_CHANNELS,
                PERM_MANAGE_ROLES,
                PERM_KICK_MEMBERS,
                PERM_BAN_MEMBERS,
                PERM_ADMINISTRATOR,
                PERM_CONNECT_VOICE,
                PERM_SPEAK_VOICE,
                PERM_MUTE_MEMBERS,
            ]),
            priority: 100,
        },
    );
    roles.insert(
        ROLE_EVERYONE.to_string(),
        RoleState {
            role_id: ROLE_EVERYONE.to_string(),
            name: "Everyone".to_string(),
            permissions: permission_bits(&[PERM_VIEW_CHANNEL, PERM_SEND_MESSAGE]),
            priority: 0,
        },
    );

    let mut member_roles = BTreeMap::new();
    member_roles.insert(
        creator.clone(),
        [ROLE_OWNER.to_string(), ROLE_EVERYONE.to_string()]
            .into_iter()
            .collect(),
    );

    GuildState {
        guild_id,
        name,
        created_by: creator,
        members,
        roles,
        member_roles,
        channels: BTreeMap::new(),
    }
}
