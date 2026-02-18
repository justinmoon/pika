use std::collections::BTreeMap;

use rapture_core::chat::{ChatApplyOutcome, ChatEnvelope, ChatState, EpochKeyLookup};

struct StaticKeys {
    keys: BTreeMap<(String, String, u64), [u8; 32]>,
}

impl StaticKeys {
    fn new(guild_id: &str, channel_id: &str, epoch: u64, key: [u8; 32]) -> Self {
        let mut keys = BTreeMap::new();
        keys.insert((guild_id.to_string(), channel_id.to_string(), epoch), key);
        Self { keys }
    }
}

impl EpochKeyLookup for StaticKeys {
    fn epoch_key(&self, guild_id: &str, channel_id: &str, epoch: u64) -> Option<[u8; 32]> {
        self.keys
            .get(&(guild_id.to_string(), channel_id.to_string(), epoch))
            .copied()
    }
}

#[test]
fn send_edit_delete_reaction_round_trip() {
    let key = [7_u8; 32];
    let lookup = StaticKeys::new("g-1", "c-1", 1, key);
    let mut state = ChatState::default();

    let send = ChatEnvelope::message_send(
        "chat-op-1".to_string(),
        1,
        "g-1".to_string(),
        "c-1".to_string(),
        "alice".to_string(),
        "m-1".to_string(),
        "hello",
        1,
        key,
    )
    .expect("send envelope");
    let out = state.apply(send, &lookup).expect("apply send");
    assert_eq!(out, ChatApplyOutcome::Applied);
    assert_eq!(state.timeline("g-1", "c-1")[0].content, "hello");

    let edit = ChatEnvelope::message_edit(
        "chat-op-2".to_string(),
        2,
        "g-1".to_string(),
        "c-1".to_string(),
        "alice".to_string(),
        "m-1".to_string(),
        "hello (edited)",
        1,
        key,
    )
    .expect("edit envelope");
    state.apply(edit, &lookup).expect("apply edit");
    assert_eq!(state.timeline("g-1", "c-1")[0].content, "hello (edited)");
    assert!(state.timeline("g-1", "c-1")[0].edited);

    let reaction = ChatEnvelope::reaction_put(
        "chat-op-3".to_string(),
        3,
        "g-1".to_string(),
        "c-1".to_string(),
        "bob".to_string(),
        "m-1".to_string(),
        ":+1:".to_string(),
    );
    state.apply(reaction, &lookup).expect("apply reaction");
    assert!(state.timeline("g-1", "c-1")[0]
        .reactions
        .contains_key(":+1:"));

    let del = ChatEnvelope::message_delete(
        "chat-op-4".to_string(),
        4,
        "g-1".to_string(),
        "c-1".to_string(),
        "alice".to_string(),
        "m-1".to_string(),
    );
    state.apply(del, &lookup).expect("apply delete");
    assert_eq!(state.timeline("g-1", "c-1")[0].content, "[deleted]");
    assert!(state.timeline("g-1", "c-1")[0].deleted);
}
