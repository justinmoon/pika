use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::control::ControlState;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChannelGroupError {
    #[error("channel group not found: {guild_id}/{channel_id}")]
    Missing {
        guild_id: String,
        channel_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelGroupState {
    pub guild_id: String,
    pub channel_id: String,
    pub epoch: u64,
    pub members: BTreeSet<String>,
    pub key: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelGroupDirectory {
    groups: BTreeMap<(String, String), ChannelGroupState>,
}

impl ChannelGroupDirectory {
    pub fn ensure_from_control(&mut self, control: &ControlState) -> usize {
        let mut created = 0_usize;
        for (guild_id, guild) in &control.guilds {
            for channel_id in guild.channels.keys() {
                let key = (guild_id.clone(), channel_id.clone());
                if self.groups.contains_key(&key) {
                    continue;
                }

                created += 1;
                self.groups.insert(
                    key,
                    ChannelGroupState {
                        guild_id: guild_id.clone(),
                        channel_id: channel_id.clone(),
                        epoch: 0,
                        members: BTreeSet::new(),
                        key: derive_initial_key(guild_id, channel_id),
                    },
                );
            }
        }
        created
    }

    pub fn get(&self, guild_id: &str, channel_id: &str) -> Option<&ChannelGroupState> {
        self.groups
            .get(&(guild_id.to_string(), channel_id.to_string()))
    }

    pub fn current_epoch_key(&self, guild_id: &str, channel_id: &str) -> Option<(u64, [u8; 32])> {
        self.get(guild_id, channel_id).map(|g| (g.epoch, g.key))
    }

    pub fn members(
        &self,
        guild_id: &str,
        channel_id: &str,
    ) -> Result<BTreeSet<String>, ChannelGroupError> {
        let group = self
            .get(guild_id, channel_id)
            .ok_or_else(|| ChannelGroupError::Missing {
                guild_id: guild_id.to_string(),
                channel_id: channel_id.to_string(),
            })?;
        Ok(group.members.clone())
    }

    pub fn reconcile_members(
        &mut self,
        guild_id: &str,
        channel_id: &str,
        desired: BTreeSet<String>,
    ) -> Result<bool, ChannelGroupError> {
        let key = (guild_id.to_string(), channel_id.to_string());
        let group = self
            .groups
            .get_mut(&key)
            .ok_or_else(|| ChannelGroupError::Missing {
                guild_id: guild_id.to_string(),
                channel_id: channel_id.to_string(),
            })?;

        if group.members == desired {
            return Ok(false);
        }

        group.members = desired;
        group.epoch = group.epoch.saturating_add(1);
        group.key = derive_rotated_key(group.key, group.epoch);
        Ok(true)
    }

    pub fn channel_refs(&self) -> Vec<(String, String)> {
        self.groups.keys().cloned().collect()
    }
}

fn derive_initial_key(guild_id: &str, channel_id: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"rapture.channel-group.init");
    h.update(guild_id.as_bytes());
    h.update([0]);
    h.update(channel_id.as_bytes());
    let out = h.finalize();
    let mut key = [0_u8; 32];
    key.copy_from_slice(&out);
    key
}

fn derive_rotated_key(previous_key: [u8; 32], epoch: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"rapture.channel-group.rotate");
    h.update(previous_key);
    h.update(epoch.to_be_bytes());
    let out = h.finalize();
    let mut key = [0_u8; 32];
    key.copy_from_slice(&out);
    key
}
