use std::collections::{BTreeMap, BTreeSet};

use crate::control::ControlState;
use crate::permissions::{has_permission, Permission};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReconcileError {
    #[error("guild not found: {0}")]
    GuildNotFound(String),
    #[error("channel not found: {guild_id}/{channel_id}")]
    ChannelNotFound {
        guild_id: String,
        channel_id: String,
    },
    #[error("membership backend error: {0}")]
    Backend(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MembershipDiff {
    pub to_add: BTreeSet<String>,
    pub to_remove: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReconcileReport {
    pub desired: BTreeSet<String>,
    pub actual_before: BTreeSet<String>,
    pub diff: MembershipDiff,
    pub added: BTreeSet<String>,
    pub removed: BTreeSet<String>,
    pub failed_add: BTreeSet<String>,
    pub failed_remove: BTreeSet<String>,
}

impl ReconcileReport {
    pub fn converged(&self) -> bool {
        self.failed_add.is_empty() && self.failed_remove.is_empty()
    }
}

pub trait MembershipBackend {
    fn actual_members(&self, guild_id: &str, channel_id: &str) -> Result<BTreeSet<String>, String>;
    fn add_member(&mut self, guild_id: &str, channel_id: &str, member: &str) -> Result<(), String>;
    fn remove_member(
        &mut self,
        guild_id: &str,
        channel_id: &str,
        member: &str,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryMembershipBackend {
    members: BTreeMap<(String, String), BTreeSet<String>>,
    fail_add_once: BTreeSet<(String, String, String)>,
    fail_remove_once: BTreeSet<(String, String, String)>,
}

impl InMemoryMembershipBackend {
    pub fn set_actual(&mut self, guild_id: &str, channel_id: &str, members: BTreeSet<String>) {
        self.members
            .insert((guild_id.to_string(), channel_id.to_string()), members);
    }

    pub fn members(&self, guild_id: &str, channel_id: &str) -> BTreeSet<String> {
        self.members
            .get(&(guild_id.to_string(), channel_id.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    pub fn fail_add_once(&mut self, guild_id: &str, channel_id: &str, member: &str) {
        self.fail_add_once.insert((
            guild_id.to_string(),
            channel_id.to_string(),
            member.to_string(),
        ));
    }

    pub fn fail_remove_once(&mut self, guild_id: &str, channel_id: &str, member: &str) {
        self.fail_remove_once.insert((
            guild_id.to_string(),
            channel_id.to_string(),
            member.to_string(),
        ));
    }
}

impl MembershipBackend for InMemoryMembershipBackend {
    fn actual_members(&self, guild_id: &str, channel_id: &str) -> Result<BTreeSet<String>, String> {
        Ok(self.members(guild_id, channel_id))
    }

    fn add_member(&mut self, guild_id: &str, channel_id: &str, member: &str) -> Result<(), String> {
        let fail_key = (
            guild_id.to_string(),
            channel_id.to_string(),
            member.to_string(),
        );
        if self.fail_add_once.remove(&fail_key) {
            return Err(format!("injected add failure for {member}"));
        }

        self.members
            .entry((guild_id.to_string(), channel_id.to_string()))
            .or_default()
            .insert(member.to_string());
        Ok(())
    }

    fn remove_member(
        &mut self,
        guild_id: &str,
        channel_id: &str,
        member: &str,
    ) -> Result<(), String> {
        let fail_key = (
            guild_id.to_string(),
            channel_id.to_string(),
            member.to_string(),
        );
        if self.fail_remove_once.remove(&fail_key) {
            return Err(format!("injected remove failure for {member}"));
        }

        if let Some(set) = self
            .members
            .get_mut(&(guild_id.to_string(), channel_id.to_string()))
        {
            set.remove(member);
        }
        Ok(())
    }
}

pub fn desired_channel_members(
    control: &ControlState,
    guild_id: &str,
    channel_id: &str,
) -> Result<BTreeSet<String>, ReconcileError> {
    let guild = control
        .guilds
        .get(guild_id)
        .ok_or_else(|| ReconcileError::GuildNotFound(guild_id.to_string()))?;
    if !guild.channels.contains_key(channel_id) {
        return Err(ReconcileError::ChannelNotFound {
            guild_id: guild_id.to_string(),
            channel_id: channel_id.to_string(),
        });
    }

    Ok(guild
        .members
        .iter()
        .filter(|member| has_permission(guild, member, Some(channel_id), Permission::ViewChannel))
        .cloned()
        .collect())
}

pub fn compute_diff(desired: &BTreeSet<String>, actual: &BTreeSet<String>) -> MembershipDiff {
    MembershipDiff {
        to_add: desired.difference(actual).cloned().collect(),
        to_remove: actual.difference(desired).cloned().collect(),
    }
}

pub fn reconcile_channel<B: MembershipBackend>(
    control: &ControlState,
    guild_id: &str,
    channel_id: &str,
    backend: &mut B,
) -> Result<ReconcileReport, ReconcileError> {
    let desired = desired_channel_members(control, guild_id, channel_id)?;
    let actual_before = backend
        .actual_members(guild_id, channel_id)
        .map_err(ReconcileError::Backend)?;
    let diff = compute_diff(&desired, &actual_before);

    let mut report = ReconcileReport {
        desired,
        actual_before,
        diff,
        ..ReconcileReport::default()
    };

    for member in report.diff.to_add.iter().cloned() {
        match backend.add_member(guild_id, channel_id, &member) {
            Ok(()) => {
                report.added.insert(member);
            }
            Err(_) => {
                report.failed_add.insert(member);
            }
        }
    }

    for member in report.diff.to_remove.iter().cloned() {
        match backend.remove_member(guild_id, channel_id, &member) {
            Ok(()) => {
                report.removed.insert(member);
            }
            Err(_) => {
                report.failed_remove.insert(member);
            }
        }
    }

    Ok(report)
}
