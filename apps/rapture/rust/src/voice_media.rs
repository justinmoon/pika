use pika_media::session::{
    InMemoryRelay, MediaFrame, MediaSession, MediaSessionError, SessionConfig,
};
use pika_media::subscription::MediaFrameSubscription;
use pika_media::tracks::{broadcast_path, default_audio_track, TrackAddress, TrackCatalog};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum VoiceMediaError {
    #[error("invalid path: {0}")]
    Path(String),
    #[error("media session error: {0}")]
    Session(#[from] MediaSessionError),
}

#[derive(Debug, Clone)]
pub struct VoiceMediaPeer {
    pub participant_label: String,
    pub publish_track: TrackAddress,
    pub catalog: TrackCatalog,
    session: MediaSession,
    broadcast_base: String,
}

impl VoiceMediaPeer {
    pub fn subscribe_to(
        &self,
        remote_participant_label: &str,
    ) -> Result<MediaFrameSubscription, VoiceMediaError> {
        let track = audio_track_for_participant(&self.broadcast_base, remote_participant_label)?;
        self.session
            .subscribe(&track)
            .map_err(VoiceMediaError::from)
    }

    pub fn publish_audio_frame(
        &self,
        seq: u64,
        timestamp_us: u64,
        payload: Vec<u8>,
    ) -> Result<usize, VoiceMediaError> {
        let frame = MediaFrame {
            seq,
            timestamp_us,
            keyframe: true,
            payload,
        };
        self.session
            .publish(&self.publish_track, frame)
            .map_err(VoiceMediaError::from)
    }
}

pub fn connect_in_memory_peer(
    relay: InMemoryRelay,
    moq_url: &str,
    relay_auth: &str,
    broadcast_base: &str,
    participant_pubkey: &str,
) -> Result<VoiceMediaPeer, VoiceMediaError> {
    let config = SessionConfig {
        moq_url: moq_url.to_string(),
        relay_auth: relay_auth.to_string(),
    };
    let mut session = MediaSession::with_relay(config, relay);
    session.connect()?;

    let participant_label = participant_label_hex(participant_pubkey);
    let publish_track = audio_track_for_participant(broadcast_base, &participant_label)?;
    let catalog = TrackCatalog::voice_default(publish_track.broadcast_path.clone());

    Ok(VoiceMediaPeer {
        participant_label,
        publish_track,
        catalog,
        session,
        broadcast_base: broadcast_base.to_string(),
    })
}

pub fn default_broadcast_base(guild_id: &str, channel_id: &str, session_id: &str) -> String {
    format!("rapture/voice/{guild_id}/{channel_id}/{session_id}")
}

pub fn participant_label_hex(pubkey: &str) -> String {
    let digest = Sha256::digest(pubkey.as_bytes());
    bytes_to_hex(&digest)
}

pub fn audio_track_for_participant(
    broadcast_base: &str,
    participant_label_hex: &str,
) -> Result<TrackAddress, VoiceMediaError> {
    let path =
        broadcast_path(broadcast_base, participant_label_hex).map_err(VoiceMediaError::Path)?;
    Ok(TrackAddress {
        broadcast_path: path,
        track_name: default_audio_track().name,
    })
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(nibble_to_hex((b >> 4) & 0x0f));
        out.push(nibble_to_hex(b & 0x0f));
    }
    out
}

fn nibble_to_hex(v: u8) -> char {
    match v {
        0..=9 => (b'0' + v) as char,
        10..=15 => (b'a' + (v - 10)) as char,
        _ => '0',
    }
}
