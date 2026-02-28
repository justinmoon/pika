use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use anyhow::Context;
use async_trait::async_trait;
use nostr_sdk::hashes::{sha256, Hash as _};
use nostr_sdk::prelude::*;
use pika_agent_control_plane::{
    AgentControlCmdEnvelope, AgentControlCommand, AgentControlErrorEnvelope,
    AgentControlResultEnvelope, AgentControlStatusEnvelope, BuildKind, CancelBuildCommand,
    GetBuildCommand, GetCapabilitiesCommand, GetRuntimeCommand, ListRuntimesCommand,
    ProcessWelcomeCommand, ProtocolKind, ProviderKind, ProvisionCommand,
    ResolveDistributionCommand, RuntimeDescriptor, RuntimeLifecyclePhase, SubmitBuildCommand,
    TeardownCommand, CMD_SCHEMA_V1, CONTROL_CMD_KIND, CONTROL_ERROR_KIND, CONTROL_RESULT_KIND,
    CONTROL_STATUS_KIND, ERROR_SCHEMA_V1, RESULT_SCHEMA_V1, STATUS_SCHEMA_V1,
};
use pika_agent_microvm::{
    build_create_vm_request, resolve_params, spawner_create_error, MicrovmSpawnerClient,
};
use pika_relay_profiles::default_message_relays;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::agent_clients::fly_machines::{DeleteMachineOutcome, DeleteVolumeOutcome, FlyClient};

const DEFAULT_CONTROL_STATE_PATH: &str = ".pika-agent-control-state.json";
const DEFAULT_CONTROL_LOOKBACK_SECS: u64 = 600;
const DEFAULT_IDEMPOTENCY_MAX_ENTRIES: usize = 8192;
const EVENT_DEDUP_WINDOW: usize = 8192;
const DEFAULT_RUNTIME_TTL_SECS: u64 = 60 * 60;
const DEFAULT_REAPER_INTERVAL_SECS: u64 = 30;
const MIN_REAPER_INTERVAL_SECS: u64 = 5;
const MAX_RETRY_DELAY_SECS: u64 = 30 * 60;
const DEFAULT_BUILD_TIMEOUT_SECS: u64 = 20 * 60;
const DEFAULT_BUILD_ARTIFACT_TTL_SECS: u64 = 6 * 60 * 60;
const DEFAULT_BUILD_MAX_ACTIVE: usize = 8;
const DEFAULT_BUILD_MAX_SUBMISSIONS_PER_HOUR: usize = 30;
const DEFAULT_BUILD_MAX_CONTEXT_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_AUDIT_MAX_ENTRIES: usize = 4096;

#[derive(Clone)]
pub struct AgentControlRuntime {
    client: Client,
    keys: Keys,
    relays: Vec<RelayUrl>,
    service: AgentControlService,
}

impl AgentControlRuntime {
    pub async fn from_env() -> anyhow::Result<Option<Self>> {
        let explicit_enabled = env_bool("PIKA_AGENT_CONTROL_ENABLED");
        let maybe_secret = std::env::var("PIKA_AGENT_CONTROL_NOSTR_SECRET")
            .ok()
            .or_else(|| std::env::var("NOSTR_SECRET_KEY").ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let enabled = explicit_enabled.unwrap_or(maybe_secret.is_some());
        if !enabled {
            return Ok(None);
        }

        let secret = maybe_secret.context(
            "agent control is enabled but no secret key found (set PIKA_AGENT_CONTROL_NOSTR_SECRET or NOSTR_SECRET_KEY)",
        )?;
        let keys = Keys::parse(&secret).context("parse agent control nostr secret key")?;

        let relay_csv = std::env::var("PIKA_AGENT_CONTROL_RELAYS")
            .ok()
            .or_else(|| std::env::var("RELAYS").ok())
            .unwrap_or_default();
        let relay_urls: Vec<String> = relay_csv
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if relay_urls.is_empty() {
            anyhow::bail!(
                "agent control is enabled but no relays are configured (set PIKA_AGENT_CONTROL_RELAYS or RELAYS)"
            );
        }
        let relays = parse_relay_urls(&relay_urls)?;

        let client = Client::new(keys.clone());
        for relay in &relays {
            client
                .add_relay(relay.clone())
                .await
                .with_context(|| format!("add agent control relay {relay}"))?;
        }
        client.connect().await;

        info!(
            pubkey = %keys.public_key(),
            relay_count = relays.len(),
            "agent control plane enabled"
        );

        Ok(Some(Self {
            client,
            keys,
            relays,
            service: AgentControlService::new()?,
        }))
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let lookback_secs = control_cmd_lookback_secs();
        let since_unix = Timestamp::now().as_secs().saturating_sub(lookback_secs);
        let filter = Filter::new()
            .kind(Kind::Custom(CONTROL_CMD_KIND))
            .custom_tag(
                SingleLetterTag::lowercase(Alphabet::P),
                self.keys.public_key().to_hex(),
            )
            .since(Timestamp::from(since_unix));
        self.client.subscribe(filter, None).await?;

        let mut notifications = self.client.notifications();
        let mut seen: HashSet<EventId> = HashSet::new();
        let mut seen_order: VecDeque<EventId> = VecDeque::new();
        let mut reaper_tick =
            tokio::time::interval(std::time::Duration::from_secs(reaper_interval_secs()));
        reaper_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            let notification = tokio::select! {
                _ = reaper_tick.tick() => {
                    if let Err(err) = self.service.reap_expired_runtimes_once().await {
                        warn!(error = %err, "agent control reaper tick failed");
                    }
                    continue;
                }
                notification = notifications.recv() => notification,
            };
            let notification = match notification {
                Ok(notification) => notification,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!(skipped, "agent control listener lagged notifications");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    anyhow::bail!("agent control listener channel closed");
                }
            };

            let RelayPoolNotification::Event { event, .. } = notification else {
                continue;
            };
            let event = *event;
            if event.kind != Kind::Custom(CONTROL_CMD_KIND) {
                continue;
            }
            if !seen.insert(event.id) {
                continue;
            }
            seen_order.push_back(event.id);
            while seen_order.len() > EVENT_DEDUP_WINDOW {
                if let Some(oldest) = seen_order.pop_front() {
                    seen.remove(&oldest);
                }
            }

            let requester = event.pubkey;
            let decrypted = match nostr_sdk::nostr::nips::nip44::decrypt(
                self.keys.secret_key(),
                &requester,
                event.content.as_str(),
            ) {
                Ok(content) => content,
                Err(err) => {
                    warn!(
                        error = %err,
                        requester = %requester,
                        "failed to decrypt control command"
                    );
                    continue;
                }
            };
            let cmd = match serde_json::from_str::<AgentControlCmdEnvelope>(&decrypted) {
                Ok(cmd) => cmd,
                Err(err) => {
                    let request_id = extract_request_id(&decrypted)
                        .unwrap_or_else(|| "unknown-request".to_string());
                    let envelope = AgentControlErrorEnvelope::v1(
                        request_id,
                        "invalid_command_json",
                        Some("command payload must decode as agent.control.cmd.v1".to_string()),
                        Some(err.to_string()),
                    );
                    if let Err(publish_err) = publish_control_event(
                        &self.client,
                        &self.keys,
                        &self.relays,
                        requester,
                        CONTROL_ERROR_KIND,
                        &envelope,
                    )
                    .await
                    {
                        error!(error = %publish_err, "failed to publish command decode error");
                    }
                    continue;
                }
            };

            let outcome = self
                .service
                .handle_command(&requester.to_hex(), requester, cmd)
                .await;

            for status in &outcome.statuses {
                if let Err(err) = publish_control_event(
                    &self.client,
                    &self.keys,
                    &self.relays,
                    requester,
                    CONTROL_STATUS_KIND,
                    status,
                )
                .await
                {
                    error!(error = %err, "failed to publish control status");
                }
            }

            if let Some(result) = &outcome.result {
                if let Err(err) = publish_control_event(
                    &self.client,
                    &self.keys,
                    &self.relays,
                    requester,
                    CONTROL_RESULT_KIND,
                    result,
                )
                .await
                {
                    error!(error = %err, "failed to publish control result");
                }
            }

            if let Some(error_envelope) = &outcome.error {
                if let Err(err) = publish_control_event(
                    &self.client,
                    &self.keys,
                    &self.relays,
                    requester,
                    CONTROL_ERROR_KIND,
                    error_envelope,
                )
                .await
                {
                    error!(error = %err, "failed to publish control error");
                }
            }
        }
    }
}

pub fn control_schema_healthcheck() -> anyhow::Result<()> {
    anyhow::ensure!(CMD_SCHEMA_V1 == "agent.control.cmd.v1");
    anyhow::ensure!(STATUS_SCHEMA_V1 == "agent.control.status.v1");
    anyhow::ensure!(RESULT_SCHEMA_V1 == "agent.control.result.v1");
    anyhow::ensure!(ERROR_SCHEMA_V1 == "agent.control.error.v1");
    Ok(())
}

fn extract_request_id(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| {
            v.get("request_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
}

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key).ok().and_then(|raw| match raw.trim() {
        "1" | "true" | "TRUE" | "yes" | "on" => Some(true),
        "0" | "false" | "FALSE" | "no" | "off" => Some(false),
        _ => None,
    })
}

fn env_usize(key: &str) -> Option<usize> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}

fn control_cmd_lookback_secs() -> u64 {
    env_u64("PIKA_AGENT_CONTROL_CMD_LOOKBACK_SECS").unwrap_or(DEFAULT_CONTROL_LOOKBACK_SECS)
}

fn runtime_ttl_secs() -> u64 {
    env_u64("PIKA_AGENT_CONTROL_RUNTIME_TTL_SECS")
        .unwrap_or(DEFAULT_RUNTIME_TTL_SECS)
        .max(60)
}

fn reaper_interval_secs() -> u64 {
    env_u64("PIKA_AGENT_CONTROL_REAPER_INTERVAL_SECS")
        .unwrap_or(DEFAULT_REAPER_INTERVAL_SECS)
        .max(MIN_REAPER_INTERVAL_SECS)
}

fn parse_relay_urls(relay_urls: &[String]) -> anyhow::Result<Vec<RelayUrl>> {
    relay_urls
        .iter()
        .map(|relay| RelayUrl::parse(relay).with_context(|| format!("parse relay url {relay}")))
        .collect()
}

async fn publish_control_event(
    client: &Client,
    keys: &Keys,
    relays: &[RelayUrl],
    recipient: PublicKey,
    kind: u16,
    payload: &impl Serialize,
) -> anyhow::Result<()> {
    let content = serde_json::to_string(payload).context("serialize control event payload")?;
    let encrypted = nostr_sdk::nostr::nips::nip44::encrypt(
        keys.secret_key(),
        &recipient,
        content,
        nostr_sdk::nostr::nips::nip44::Version::V2,
    )
    .context("encrypt control event payload")?;
    let event = EventBuilder::new(Kind::Custom(kind), encrypted)
        .tags([Tag::public_key(recipient)])
        .sign_with_keys(keys)
        .context("sign control event")?;
    let output = client
        .send_event_to(relays.to_vec(), &event)
        .await
        .context("publish control event")?;
    if output.success.is_empty() {
        let reasons: Vec<String> = output.failed.values().cloned().collect();
        anyhow::bail!("no relay accepted control event kind={kind}: {reasons:?}");
    }
    Ok(())
}

#[derive(Clone, Debug)]
enum ProvisionPolicy {
    AllowAll,
    Allowlist(HashSet<String>),
    DenyAll,
}

impl ProvisionPolicy {
    fn is_allowed(&self, requester_pubkey_hex: &str) -> bool {
        match self {
            Self::AllowAll => true,
            Self::Allowlist(allowed) => allowed.contains(requester_pubkey_hex),
            Self::DenyAll => false,
        }
    }
}

fn load_provision_policy() -> anyhow::Result<ProvisionPolicy> {
    if env_bool("PIKA_AGENT_CONTROL_ALLOW_OPEN_PROVISIONING") == Some(true) {
        warn!("PIKA_AGENT_CONTROL_ALLOW_OPEN_PROVISIONING=1 set; any requester may provision");
        return Ok(ProvisionPolicy::AllowAll);
    }

    let raw = std::env::var("PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST")
        .ok()
        .unwrap_or_default();
    let mut allowed = HashSet::new();
    for value in raw.split(',') {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let pubkey = PublicKey::parse(trimmed)
            .with_context(|| format!("parse provision allowlist pubkey: {trimmed}"))?;
        allowed.insert(pubkey.to_hex());
    }
    if allowed.is_empty() {
        warn!(
            "no provisioning allowlist configured; provisioning commands are disabled (set PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST or PIKA_AGENT_CONTROL_ALLOW_OPEN_PROVISIONING=1)"
        );
        Ok(ProvisionPolicy::DenyAll)
    } else {
        info!(count = allowed.len(), "loaded provisioning allowlist");
        Ok(ProvisionPolicy::Allowlist(allowed))
    }
}

#[derive(Clone, Debug)]
struct V2RolloutFlags {
    advanced_workload_enabled: bool,
}

fn load_v2_rollout_flags() -> V2RolloutFlags {
    V2RolloutFlags {
        advanced_workload_enabled: env_bool("PIKA_AGENT_CONTROL_V2_ADVANCED_WORKLOAD_ENABLED")
            == Some(true),
    }
}

#[derive(Clone, Debug)]
struct SourceAllowRule {
    source_prefix: Option<String>,
    scheme: String,
    host: String,
    port: Option<u16>,
    path_segments: Vec<String>,
}

#[derive(Clone, Debug)]
struct ParsedSourceRef {
    source_prefix: Option<String>,
    scheme: String,
    host: String,
    port: Option<u16>,
    path_segments: Vec<String>,
}

impl SourceAllowRule {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        let parsed = parse_source_ref(raw)?;
        Ok(Self {
            source_prefix: parsed.source_prefix,
            scheme: parsed.scheme,
            host: parsed.host,
            port: parsed.port,
            path_segments: parsed.path_segments,
        })
    }

    fn matches(&self, source: &ParsedSourceRef) -> bool {
        if self.source_prefix != source.source_prefix {
            return false;
        }
        if self.scheme != source.scheme {
            return false;
        }
        if self.host != source.host {
            return false;
        }
        if self.port != source.port {
            return false;
        }
        if self.path_segments.len() > source.path_segments.len() {
            return false;
        }
        self.path_segments
            .iter()
            .zip(source.path_segments.iter())
            .all(|(lhs, rhs)| lhs == rhs)
    }
}

#[derive(Clone, Debug)]
struct BuildPolicy {
    max_active_builds: usize,
    max_submissions_per_hour: usize,
    max_context_bytes: u64,
    default_timeout_secs: u64,
    max_timeout_secs: u64,
    artifact_ttl_secs: u64,
    max_audit_entries: usize,
    allowed_source_rules: Vec<SourceAllowRule>,
}

fn load_build_policy() -> BuildPolicy {
    let raw_allowed = std::env::var("PIKA_AGENT_CONTROL_BUILD_ALLOWED_SOURCE_PREFIXES")
        .ok()
        .unwrap_or_default();
    let mut allowed_source_rules = Vec::new();
    for entry in raw_allowed.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        match SourceAllowRule::parse(trimmed) {
            Ok(rule) => allowed_source_rules.push(rule),
            Err(err) => warn!(
                error = %err,
                source_rule = trimmed,
                "ignoring invalid source allowlist rule"
            ),
        }
    }
    BuildPolicy {
        max_active_builds: env_usize("PIKA_AGENT_CONTROL_BUILD_MAX_ACTIVE")
            .unwrap_or(DEFAULT_BUILD_MAX_ACTIVE)
            .max(1),
        max_submissions_per_hour: env_usize("PIKA_AGENT_CONTROL_BUILD_MAX_SUBMISSIONS_PER_HOUR")
            .unwrap_or(DEFAULT_BUILD_MAX_SUBMISSIONS_PER_HOUR)
            .max(1),
        max_context_bytes: env_u64("PIKA_AGENT_CONTROL_BUILD_MAX_CONTEXT_BYTES")
            .unwrap_or(DEFAULT_BUILD_MAX_CONTEXT_BYTES)
            .max(1024),
        default_timeout_secs: env_u64("PIKA_AGENT_CONTROL_BUILD_DEFAULT_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_BUILD_TIMEOUT_SECS)
            .max(60),
        max_timeout_secs: env_u64("PIKA_AGENT_CONTROL_BUILD_MAX_TIMEOUT_SECS")
            .unwrap_or(DEFAULT_BUILD_TIMEOUT_SECS.saturating_mul(4))
            .max(60),
        artifact_ttl_secs: env_u64("PIKA_AGENT_CONTROL_BUILD_ARTIFACT_TTL_SECS")
            .unwrap_or(DEFAULT_BUILD_ARTIFACT_TTL_SECS)
            .max(60),
        max_audit_entries: env_usize("PIKA_AGENT_CONTROL_AUDIT_MAX_ENTRIES")
            .unwrap_or(DEFAULT_AUDIT_MAX_ENTRIES)
            .max(128),
        allowed_source_rules,
    }
}

#[derive(Clone)]
struct AgentControlService {
    state: std::sync::Arc<RwLock<ControlState>>,
    persistence: Option<std::sync::Arc<ControlStatePersistence>>,
    provision_policy: ProvisionPolicy,
    v2_flags: V2RolloutFlags,
    build_policy: BuildPolicy,
    idempotency_max_entries: usize,
    fly: std::sync::Arc<dyn ProviderAdapter>,
    microvm: std::sync::Arc<dyn ProviderAdapter>,
    build_service: std::sync::Arc<dyn BuildServiceAdapter>,
}

impl AgentControlService {
    fn new() -> anyhow::Result<Self> {
        let state_path = std::env::var("PIKA_AGENT_CONTROL_STATE_PATH")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONTROL_STATE_PATH));
        let idempotency_max_entries = env_usize("PIKA_AGENT_CONTROL_IDEMPOTENCY_MAX_ENTRIES")
            .unwrap_or(DEFAULT_IDEMPOTENCY_MAX_ENTRIES)
            .max(256);
        let persistence = std::sync::Arc::new(ControlStatePersistence::new(state_path));
        let mut loaded_state = persistence.load()?;
        loaded_state.truncate_idempotency(idempotency_max_entries);
        let lease_migrations =
            loaded_state.ensure_runtime_lease_fields(unix_now_secs(), runtime_ttl_secs());
        let build_migrations =
            loaded_state.ensure_v2_build_fields(unix_now_secs(), DEFAULT_BUILD_ARTIFACT_TTL_SECS);
        if lease_migrations > 0 {
            warn!(
                count = lease_migrations,
                "backfilled legacy runtime lease fields while loading control state"
            );
            if let Err(err) = persistence.save(&loaded_state) {
                warn!(
                    error = %err,
                    "failed to persist backfilled runtime lease field migration"
                );
            }
        }
        if build_migrations > 0 {
            warn!(
                count = build_migrations,
                "backfilled legacy build/artifact fields while loading control state"
            );
            if let Err(err) = persistence.save(&loaded_state) {
                warn!(
                    error = %err,
                    "failed to persist backfilled build/artifact migration"
                );
            }
        }
        let v2_flags = load_v2_rollout_flags();
        let build_policy = load_build_policy();
        info!(
            runtimes = loaded_state.runtimes.len(),
            builds = loaded_state.builds.len(),
            idempotency = loaded_state.idempotency.len(),
            path = %persistence.path.display(),
            "loaded agent control state"
        );
        let provision_policy = load_provision_policy()?;
        Ok(Self {
            state: std::sync::Arc::new(RwLock::new(loaded_state)),
            persistence: Some(persistence),
            provision_policy,
            v2_flags,
            build_policy,
            idempotency_max_entries,
            fly: std::sync::Arc::new(FlyAdapter),
            microvm: std::sync::Arc::new(MicrovmAdapter),
            build_service: std::sync::Arc::new(DefaultBuildService),
        })
    }

    #[cfg(test)]
    fn with_adapters(
        fly: std::sync::Arc<dyn ProviderAdapter>,
        microvm: std::sync::Arc<dyn ProviderAdapter>,
    ) -> Self {
        Self {
            state: std::sync::Arc::new(RwLock::new(ControlState::default())),
            persistence: None,
            provision_policy: ProvisionPolicy::AllowAll,
            v2_flags: V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            build_policy: load_build_policy(),
            idempotency_max_entries: DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            fly,
            microvm,
            build_service: std::sync::Arc::new(DefaultBuildService),
        }
    }

    #[cfg(test)]
    fn with_adapters_and_policy(
        fly: std::sync::Arc<dyn ProviderAdapter>,
        microvm: std::sync::Arc<dyn ProviderAdapter>,
        provision_policy: ProvisionPolicy,
        idempotency_max_entries: usize,
    ) -> Self {
        Self {
            state: std::sync::Arc::new(RwLock::new(ControlState::default())),
            persistence: None,
            provision_policy,
            v2_flags: V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            build_policy: load_build_policy(),
            idempotency_max_entries,
            fly,
            microvm,
            build_service: std::sync::Arc::new(DefaultBuildService),
        }
    }

    #[cfg(test)]
    fn with_adapters_policy_and_flags(
        fly: std::sync::Arc<dyn ProviderAdapter>,
        microvm: std::sync::Arc<dyn ProviderAdapter>,
        provision_policy: ProvisionPolicy,
        idempotency_max_entries: usize,
        v2_flags: V2RolloutFlags,
    ) -> Self {
        Self {
            state: std::sync::Arc::new(RwLock::new(ControlState::default())),
            persistence: None,
            provision_policy,
            v2_flags,
            build_policy: load_build_policy(),
            idempotency_max_entries,
            fly,
            microvm,
            build_service: std::sync::Arc::new(DefaultBuildService),
        }
    }

    #[cfg(test)]
    fn with_adapters_policy_flags_and_build_policy(
        fly: std::sync::Arc<dyn ProviderAdapter>,
        microvm: std::sync::Arc<dyn ProviderAdapter>,
        provision_policy: ProvisionPolicy,
        idempotency_max_entries: usize,
        v2_flags: V2RolloutFlags,
        build_policy: BuildPolicy,
    ) -> Self {
        Self {
            state: std::sync::Arc::new(RwLock::new(ControlState::default())),
            persistence: None,
            provision_policy,
            v2_flags,
            build_policy,
            idempotency_max_entries,
            fly,
            microvm,
            build_service: std::sync::Arc::new(DefaultBuildService),
        }
    }

    #[cfg(test)]
    fn with_adapters_policy_and_persistence(
        fly: std::sync::Arc<dyn ProviderAdapter>,
        microvm: std::sync::Arc<dyn ProviderAdapter>,
        provision_policy: ProvisionPolicy,
        idempotency_max_entries: usize,
        persistence: std::sync::Arc<ControlStatePersistence>,
    ) -> Self {
        Self {
            state: std::sync::Arc::new(RwLock::new(ControlState::default())),
            persistence: Some(persistence),
            provision_policy,
            v2_flags: V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            build_policy: load_build_policy(),
            idempotency_max_entries,
            fly,
            microvm,
            build_service: std::sync::Arc::new(DefaultBuildService),
        }
    }

    #[cfg(test)]
    fn with_adapters_policy_and_loaded_persistence(
        fly: std::sync::Arc<dyn ProviderAdapter>,
        microvm: std::sync::Arc<dyn ProviderAdapter>,
        provision_policy: ProvisionPolicy,
        idempotency_max_entries: usize,
        persistence: std::sync::Arc<ControlStatePersistence>,
    ) -> anyhow::Result<Self> {
        let mut loaded_state = persistence.load()?;
        loaded_state.truncate_idempotency(idempotency_max_entries);
        loaded_state.ensure_runtime_lease_fields(unix_now_secs(), runtime_ttl_secs());
        loaded_state.ensure_v2_build_fields(unix_now_secs(), DEFAULT_BUILD_ARTIFACT_TTL_SECS);
        Ok(Self {
            state: std::sync::Arc::new(RwLock::new(loaded_state)),
            persistence: Some(persistence),
            provision_policy,
            v2_flags: V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            build_policy: load_build_policy(),
            idempotency_max_entries,
            fly,
            microvm,
            build_service: std::sync::Arc::new(DefaultBuildService),
        })
    }

    fn persist_state_snapshot(&self, state: &ControlState) -> anyhow::Result<()> {
        let Some(persistence) = &self.persistence else {
            return Ok(());
        };
        persistence.save(state)
    }

    fn adapter_for(&self, provider: ProviderKind) -> std::sync::Arc<dyn ProviderAdapter> {
        match provider {
            ProviderKind::Fly => self.fly.clone(),
            ProviderKind::Microvm => self.microvm.clone(),
        }
    }

    fn can_access_runtime(&self, requester_pubkey_hex: &str, runtime: &RuntimeRecord) -> bool {
        if runtime.owner_pubkey_hex.is_empty() {
            // Legacy state files (pre-owner field) remain manageable by trusted operators.
            return self.provision_policy.is_allowed(requester_pubkey_hex);
        }
        runtime.owner_pubkey_hex == requester_pubkey_hex
    }

    fn push_audit_event(
        &self,
        state: &mut ControlState,
        actor_pubkey_hex: &str,
        action: &str,
        outcome: &str,
        detail: Value,
    ) {
        state.audit_log.push_back(AuditEvent {
            ts: unix_now_secs(),
            actor_pubkey_hex: actor_pubkey_hex.to_string(),
            action: action.to_string(),
            outcome: outcome.to_string(),
            detail,
        });
        while state.audit_log.len() > self.build_policy.max_audit_entries {
            state.audit_log.pop_front();
        }
    }

    async fn handle_command(
        &self,
        requester_pubkey_hex: &str,
        requester_pubkey: PublicKey,
        cmd: AgentControlCmdEnvelope,
    ) -> CommandOutcome {
        let mut statuses = vec![AgentControlStatusEnvelope::v1(
            cmd.request_id.clone(),
            RuntimeLifecyclePhase::Queued,
            None,
            None,
            Some("request queued".to_string()),
            Value::Null,
        )];

        if cmd.schema != CMD_SCHEMA_V1 {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    cmd.request_id,
                    "invalid_schema",
                    Some(format!("expected {CMD_SCHEMA_V1}")),
                    Some(format!("got {}", cmd.schema)),
                ),
            );
        }

        let cache_key = (
            requester_pubkey_hex.to_string(),
            cmd.idempotency_key.clone(),
        );
        {
            let state = self.state.read().await;
            if let Some(cached) = state.idempotency.get(&cache_key) {
                info!(
                    request_id = %cmd.request_id,
                    idempotency_key = %cmd.idempotency_key,
                    "replaying idempotent command"
                );
                statuses.push(AgentControlStatusEnvelope::v1(
                    cmd.request_id.clone(),
                    RuntimeLifecyclePhase::Ready,
                    cached.runtime_id(),
                    cached.provider(),
                    Some("idempotent replay".to_string()),
                    Value::Null,
                ));
                return cached.to_outcome(statuses, cmd.request_id);
            }
        }

        let outcome = match cmd.command.clone() {
            AgentControlCommand::Provision(provision) => {
                self.handle_provision(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    requester_pubkey,
                    provision,
                    statuses,
                )
                .await
            }
            AgentControlCommand::ProcessWelcome(process_welcome) => {
                self.handle_process_welcome(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    process_welcome,
                    statuses,
                )
                .await
            }
            AgentControlCommand::Teardown(teardown) => {
                self.handle_teardown(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    teardown,
                    statuses,
                )
                .await
            }
            AgentControlCommand::GetRuntime(get_runtime) => {
                self.handle_get_runtime(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    get_runtime,
                    statuses,
                )
                .await
            }
            AgentControlCommand::ListRuntimes(list) => {
                self.handle_list_runtimes(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    list,
                    statuses,
                )
                .await
            }
            AgentControlCommand::GetCapabilities(get_capabilities) => {
                self.handle_get_capabilities(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    get_capabilities,
                    statuses,
                )
                .await
            }
            AgentControlCommand::ResolveDistribution(resolve_distribution) => {
                self.handle_resolve_distribution(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    resolve_distribution,
                    statuses,
                )
                .await
            }
            AgentControlCommand::SubmitBuild(submit_build) => {
                self.handle_submit_build(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    submit_build,
                    statuses,
                )
                .await
            }
            AgentControlCommand::GetBuild(get_build) => {
                self.handle_get_build(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    get_build,
                    statuses,
                )
                .await
            }
            AgentControlCommand::CancelBuild(cancel_build) => {
                self.handle_cancel_build(
                    requester_pubkey_hex,
                    cmd.request_id.clone(),
                    cancel_build,
                    statuses,
                )
                .await
            }
        };

        if outcome.result.is_none() || !should_cache_success_result(&cmd.command) {
            return outcome;
        }

        let Some(result) = &outcome.result else {
            return outcome;
        };
        let terminal = CachedTerminal::Result {
            provider: result.runtime.provider,
            runtime_id: result.runtime.runtime_id.clone(),
            runtime: Box::new(result.runtime.clone()),
            payload: result.payload.clone(),
        };

        let mut state = self.state.write().await;
        state.insert_idempotency(cache_key, terminal, self.idempotency_max_entries);
        if let Err(err) = self.persist_state_snapshot(&state) {
            error!(
                error = %err,
                request_id = %cmd.request_id,
                "failed to persist idempotency cache; continuing without durable replay entry"
            );
        }
        outcome
    }

    async fn handle_provision(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        requester_pubkey: PublicKey,
        provision: ProvisionCommand,
        mut statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        if !self.provision_policy.is_allowed(requester_pubkey_hex) {
            statuses.push(AgentControlStatusEnvelope::v1(
                request_id.clone(),
                RuntimeLifecyclePhase::Failed,
                None,
                Some(provision.provider),
                Some("requester is not allowed to provision runtimes".to_string()),
                Value::Null,
            ));
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "provision_unauthorized",
                    Some(
                        "set PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST to include this requester pubkey"
                            .to_string(),
                    ),
                    Some(format!("requester_pubkey={requester_pubkey_hex}")),
                ),
            );
        }
        let mut effective_provision = provision.clone();
        if let Some(advanced) = effective_provision.advanced_workload_json.as_deref() {
            if !self.v2_flags.advanced_workload_enabled {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "v2_advanced_workload_disabled",
                        Some(
                            "set PIKA_AGENT_CONTROL_V2_ADVANCED_WORKLOAD_ENABLED=1 to enable advanced workloads"
                                .to_string(),
                        ),
                        None,
                    ),
                );
            }
            if let Err(err) = serde_json::from_str::<Value>(advanced) {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "advanced_workload_invalid_json",
                        Some("advanced_workload_json must be valid JSON".to_string()),
                        Some(err.to_string()),
                    ),
                );
            }
        }
        if effective_provision.build_id.is_some() && effective_provision.artifact_ref.is_some() {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "provision_artifact_ambiguous",
                    Some("set either build_id or artifact_ref, not both".to_string()),
                    None,
                ),
            );
        }
        let mut provision_build_id: Option<String> = None;
        if let Some(build_id) = effective_provision.build_id.as_deref() {
            let pre_refresh = {
                let state = self.state.read().await;
                state.builds.get(build_id).cloned()
            };
            let Some(pre_refresh) = pre_refresh else {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_not_found",
                        Some("build id is unknown to this server".to_string()),
                        Some(build_id.to_string()),
                    ),
                );
            };
            if pre_refresh.owner_pubkey_hex != requester_pubkey_hex {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_not_found",
                        Some("build id is unknown to this server".to_string()),
                        Some(build_id.to_string()),
                    ),
                );
            }

            let build = match self
                .refresh_build_progress(&pre_refresh.build_id, unix_now_secs())
                .await
            {
                Ok(Some(build)) => build,
                Ok(None) => {
                    return CommandOutcome::error(
                        statuses,
                        AgentControlErrorEnvelope::v1(
                            request_id,
                            "build_not_found",
                            Some("build id is unknown to this server".to_string()),
                            Some(build_id.to_string()),
                        ),
                    );
                }
                Err(err) => {
                    return CommandOutcome::error(
                        statuses,
                        AgentControlErrorEnvelope::v1(
                            request_id,
                            "build_refresh_failed",
                            Some("failed to load build state".to_string()),
                            Some(err.to_string()),
                        ),
                    );
                }
            };
            if build.phase != BuildPhase::Succeeded {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_not_ready",
                        Some("wait for build to succeed before provisioning".to_string()),
                        Some(format!("phase={:?}", build.phase)),
                    ),
                );
            }
            let Some(artifact_ref) = build.artifact_ref.clone() else {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_artifact_missing",
                        Some("build did not produce an artifact".to_string()),
                        Some(build.build_id),
                    ),
                );
            };
            effective_provision.artifact_ref = Some(artifact_ref);
            provision_build_id = Some(build_id.to_string());
        }
        if let Some(artifact_ref) = effective_provision.artifact_ref.as_deref() {
            let expected_kind = default_build_kind_for_provider(effective_provision.provider);
            if let Err(err) = ensure_immutable_artifact_ref(expected_kind, artifact_ref) {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "artifact_ref_invalid",
                        Some(
                            "artifact_ref must be immutable and compatible with provider"
                                .to_string(),
                        ),
                        Some(err.to_string()),
                    ),
                );
            }
            let parsed_kind = parse_artifact_kind(artifact_ref);
            if parsed_kind != Some(expected_kind) {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "artifact_kind_incompatible",
                        Some("artifact_ref type is incompatible with provider".to_string()),
                        Some(format!(
                            "provider={}, expected_kind={}, artifact_ref={artifact_ref}",
                            provider_name(effective_provision.provider),
                            build_kind_name(expected_kind),
                        )),
                    ),
                );
            }
        }
        statuses.push(AgentControlStatusEnvelope::v1(
            request_id.clone(),
            RuntimeLifecyclePhase::Provisioning,
            None,
            Some(effective_provision.provider),
            Some("provisioning runtime".to_string()),
            json!({
                "provider": provider_name(effective_provision.provider),
                "protocol": protocol_name(effective_provision.protocol),
                "build_id": provision_build_id.clone(),
                "artifact_ref": effective_provision.artifact_ref.clone(),
            }),
        ));
        if let Some(requested_class) = effective_provision.runtime_class.as_deref() {
            let advertised_class = runtime_profile(effective_provision.provider)
                .runtime_class
                .unwrap_or_else(|| provider_name(effective_provision.provider).to_string());
            if requested_class != advertised_class {
                statuses.push(AgentControlStatusEnvelope::v1(
                    request_id.clone(),
                    RuntimeLifecyclePhase::Failed,
                    None,
                    Some(effective_provision.provider),
                    Some("requested runtime class is not available on this server".to_string()),
                    json!({
                        "requested_runtime_class": requested_class,
                        "available_runtime_class": advertised_class,
                    }),
                ));
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "runtime_class_unavailable",
                        Some(
                            "route this command to a server that advertises the requested class"
                                .to_string(),
                        ),
                        Some(format!(
                            "requested={}, available={}",
                            requested_class, advertised_class
                        )),
                    ),
                );
            }
        }

        let runtime_id = new_runtime_id(effective_provision.provider);
        let adapter = self.adapter_for(effective_provision.provider);
        let provisioned = match adapter
            .provision(&runtime_id, requester_pubkey, &effective_provision)
            .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                statuses.push(AgentControlStatusEnvelope::v1(
                    request_id.clone(),
                    RuntimeLifecyclePhase::Failed,
                    Some(runtime_id),
                    Some(effective_provision.provider),
                    Some("provisioning failed".to_string()),
                    Value::Null,
                ));
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "provision_failed",
                        Some("check provider credentials/config and retry".to_string()),
                        Some(format!("{err:#}")),
                    ),
                );
            }
        };
        let mut runtime_metadata = provisioned.metadata.clone();
        if let Some(artifact_ref) = effective_provision.artifact_ref.clone() {
            runtime_metadata = with_artifact_metadata(
                runtime_metadata,
                provision_build_id.as_deref(),
                &artifact_ref,
            );
        }
        if let Some(advanced) = effective_provision.advanced_workload_json.as_deref() {
            if let Ok(parsed) = serde_json::from_str::<Value>(advanced) {
                runtime_metadata = with_advanced_workload_metadata(runtime_metadata, parsed);
            }
        }
        let descriptor = RuntimeDescriptor {
            runtime_id: runtime_id.clone(),
            provider: effective_provision.provider,
            lifecycle_phase: RuntimeLifecyclePhase::Ready,
            runtime_class: provisioned.runtime_class.clone(),
            region: provisioned.region.clone(),
            capacity: provisioned.capacity.clone(),
            policy_constraints: provisioned.policy_constraints.clone(),
            protocol_compatibility: provisioned.protocol_compatibility.clone(),
            bot_pubkey: provisioned.bot_pubkey.clone(),
            metadata: runtime_metadata,
        };
        let created_at = unix_now_secs();
        let expires_at = created_at.saturating_add(runtime_ttl_secs());

        let runtime_record = RuntimeRecord {
            owner_pubkey_hex: requester_pubkey_hex.to_string(),
            descriptor: descriptor.clone(),
            provider_handle: provisioned.provider_handle,
            created_at,
            expires_at,
            teardown_retry: None,
        };

        if !descriptor
            .protocol_compatibility
            .contains(&effective_provision.protocol)
        {
            statuses.push(AgentControlStatusEnvelope::v1(
                request_id.clone(),
                RuntimeLifecyclePhase::Failed,
                Some(runtime_id),
                Some(effective_provision.provider),
                Some("requested protocol is not supported by runtime".to_string()),
                json!({
                    "requested_protocol": protocol_name(effective_provision.protocol),
                }),
            ));
            let cleanup_outcome = match adapter.teardown(&runtime_record).await {
                Ok(payload) => format!("provider cleanup attempted: {payload}"),
                Err(cleanup_err) => {
                    format!("provider cleanup failed: {cleanup_err:#}")
                }
            };
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "unsupported_protocol",
                    Some("choose a compatible runtime protocol".to_string()),
                    Some(format!(
                        "requested={}, compatibility={:?}; {cleanup_outcome}",
                        protocol_name(effective_provision.protocol),
                        descriptor.protocol_compatibility
                    )),
                ),
            );
        }
        if let Some(requested_class) = effective_provision.runtime_class.as_deref() {
            if descriptor.runtime_class.as_deref() != Some(requested_class) {
                let available = descriptor
                    .runtime_class
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());
                statuses.push(AgentControlStatusEnvelope::v1(
                    request_id.clone(),
                    RuntimeLifecyclePhase::Failed,
                    Some(runtime_id),
                    Some(effective_provision.provider),
                    Some("requested runtime class is not available on this server".to_string()),
                    json!({
                        "requested_runtime_class": requested_class,
                        "available_runtime_class": available,
                    }),
                ));
                let cleanup_outcome = match adapter.teardown(&runtime_record).await {
                    Ok(payload) => format!("provider cleanup attempted: {payload}"),
                    Err(cleanup_err) => {
                        format!("provider cleanup failed: {cleanup_err:#}")
                    }
                };
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "runtime_class_unavailable",
                        Some(
                            "route this command to a server that advertises the requested class"
                                .to_string(),
                        ),
                        Some(format!(
                            "requested={}, available={}; {cleanup_outcome}",
                            requested_class, available,
                        )),
                    ),
                );
            }
        }

        {
            let mut state = self.state.write().await;
            let rollback_artifact_ref = effective_provision.artifact_ref.clone();
            let previous_artifact = rollback_artifact_ref
                .as_ref()
                .and_then(|artifact_ref| state.artifacts.get(artifact_ref))
                .cloned();
            state
                .runtimes
                .insert(runtime_id.clone(), runtime_record.clone());
            if let Some(artifact_ref) = effective_provision.artifact_ref.as_deref() {
                let artifact = state
                    .artifacts
                    .entry(artifact_ref.to_string())
                    .or_insert_with(|| ArtifactRecord {
                        artifact_ref: artifact_ref.to_string(),
                        build_kind: default_build_kind_for_provider(effective_provision.provider),
                        owner_pubkey_hex: requester_pubkey_hex.to_string(),
                        created_at,
                        last_used_at: created_at,
                        expires_at: created_at.saturating_add(self.build_policy.artifact_ttl_secs),
                        source_build_id: provision_build_id.clone(),
                    });
                artifact.last_used_at = created_at;
                if artifact.expires_at < created_at {
                    artifact.expires_at =
                        created_at.saturating_add(self.build_policy.artifact_ttl_secs);
                }
            }
            self.push_audit_event(
                &mut state,
                requester_pubkey_hex,
                "provision",
                "ok",
                json!({
                    "runtime_id": runtime_id.clone(),
                    "provider": provider_name(effective_provision.provider),
                    "artifact_ref": effective_provision.artifact_ref.clone(),
                    "build_id": provision_build_id.clone(),
                }),
            );
            if let Err(err) = self.persist_state_snapshot(&state) {
                state.runtimes.remove(&runtime_id);
                if let Some(artifact_ref) = rollback_artifact_ref {
                    if let Some(previous) = previous_artifact {
                        state.artifacts.insert(artifact_ref, previous);
                    } else {
                        state.artifacts.remove(&artifact_ref);
                    }
                }
                let rollback_err = self.persist_state_snapshot(&state).err();
                drop(state);

                let cleanup_outcome = match adapter.teardown(&runtime_record).await {
                    Ok(payload) => format!("provider rollback attempted: {payload}"),
                    Err(cleanup_err) => {
                        format!("provider rollback failed: {cleanup_err:#}")
                    }
                };
                let mut detail = format!("{err:#}; {cleanup_outcome}");
                if let Some(rollback_err) = rollback_err {
                    detail.push_str(&format!(
                        "; rollback state persist failed: {rollback_err:#}"
                    ));
                }
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "state_persist_failed",
                        Some(
                            "runtime provisioning was rolled back due to server state persistence failure"
                                .to_string(),
                        ),
                        Some(detail),
                    ),
                );
            }
        }

        statuses.push(AgentControlStatusEnvelope::v1(
            request_id.clone(),
            RuntimeLifecyclePhase::Ready,
            Some(runtime_id.clone()),
            Some(effective_provision.provider),
            Some("runtime ready".to_string()),
            json!({
                "runtime_id": runtime_id,
                "provider": provider_name(effective_provision.provider),
                "build_id": provision_build_id.clone(),
                "artifact_ref": effective_provision.artifact_ref.clone(),
                "created_at": created_at,
                "expires_at": expires_at,
            }),
        ));

        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(
                request_id,
                runtime_record.descriptor_with_lease(),
                json!({
                    "operation": "provision",
                    "build_id": provision_build_id,
                    "artifact_ref": effective_provision.artifact_ref,
                    "created_at": created_at,
                    "expires_at": expires_at,
                }),
            ),
        )
    }

    async fn handle_process_welcome(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        process_welcome: ProcessWelcomeCommand,
        mut statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        let runtime = {
            let state = self.state.read().await;
            state.runtimes.get(&process_welcome.runtime_id).cloned()
        };
        let Some(runtime) = runtime else {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "runtime_not_found",
                    Some("runtime id is unknown to this server".to_string()),
                    Some(process_welcome.runtime_id),
                ),
            );
        };
        if !self.can_access_runtime(requester_pubkey_hex, &runtime) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "runtime_not_found",
                    Some("runtime id is unknown to this server".to_string()),
                    Some(process_welcome.runtime_id),
                ),
            );
        }

        statuses.push(AgentControlStatusEnvelope::v1(
            request_id.clone(),
            RuntimeLifecyclePhase::Provisioning,
            Some(runtime.descriptor.runtime_id.clone()),
            Some(runtime.descriptor.provider),
            Some("processing welcome".to_string()),
            Value::Null,
        ));
        let adapter = self.adapter_for(runtime.descriptor.provider);
        let payload = match adapter
            .process_welcome(&runtime, &process_welcome)
            .await
            .with_context(|| "provider process_welcome call failed")
        {
            Ok(payload) => payload,
            Err(err) => {
                statuses.push(AgentControlStatusEnvelope::v1(
                    request_id.clone(),
                    RuntimeLifecyclePhase::Failed,
                    Some(runtime.descriptor.runtime_id.clone()),
                    Some(runtime.descriptor.provider),
                    Some("process_welcome failed".to_string()),
                    Value::Null,
                ));
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "process_welcome_failed",
                        Some("check provider runtime state and welcome payload".to_string()),
                        Some(format!("{err:#}")),
                    ),
                );
            }
        };
        statuses.push(AgentControlStatusEnvelope::v1(
            request_id.clone(),
            RuntimeLifecyclePhase::Ready,
            Some(runtime.descriptor.runtime_id.clone()),
            Some(runtime.descriptor.provider),
            Some("welcome processed".to_string()),
            Value::Null,
        ));
        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(request_id, runtime.descriptor_with_lease(), payload),
        )
    }

    async fn handle_teardown(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        teardown: TeardownCommand,
        mut statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        let runtime = {
            let state = self.state.read().await;
            state.runtimes.get(&teardown.runtime_id).cloned()
        };
        let Some(mut runtime) = runtime else {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "runtime_not_found",
                    Some("runtime id is unknown to this server".to_string()),
                    Some(teardown.runtime_id),
                ),
            );
        };
        if !self.can_access_runtime(requester_pubkey_hex, &runtime) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "runtime_not_found",
                    Some("runtime id is unknown to this server".to_string()),
                    Some(teardown.runtime_id),
                ),
            );
        }

        statuses.push(AgentControlStatusEnvelope::v1(
            request_id.clone(),
            RuntimeLifecyclePhase::Teardown,
            Some(runtime.descriptor.runtime_id.clone()),
            Some(runtime.descriptor.provider),
            Some("teardown in progress".to_string()),
            Value::Null,
        ));
        let previous_attempt_count = runtime
            .teardown_retry
            .as_ref()
            .map(|metadata| metadata.attempt_count)
            .unwrap_or(0);
        runtime.descriptor.lifecycle_phase = RuntimeLifecyclePhase::Teardown;
        let now = unix_now_secs();
        runtime.teardown_retry = None;
        let mut transition_persist_error: Option<String> = None;
        {
            let mut state = self.state.write().await;
            state
                .runtimes
                .insert(runtime.descriptor.runtime_id.clone(), runtime.clone());
            if let Err(err) = self.persist_state_snapshot(&state) {
                transition_persist_error = Some(format!("{err:#}"));
                warn!(
                    error = %err,
                    runtime_id = %runtime.descriptor.runtime_id,
                    "failed to persist teardown transition; continuing with in-memory state"
                );
            }
        }

        let adapter = self.adapter_for(runtime.descriptor.provider);
        let mut payload = match adapter.teardown(&runtime).await {
            Ok(payload) => payload,
            Err(err) => {
                let attempt_count = previous_attempt_count.saturating_add(1);
                let backoff_secs = teardown_backoff_secs(attempt_count);
                runtime.teardown_retry = Some(TeardownRetryMetadata {
                    attempt_count,
                    last_error: Some(format!("{err:#}")),
                    next_retry_at: Some(now.saturating_add(backoff_secs)),
                    last_attempt_at: Some(now),
                });
                {
                    let mut state = self.state.write().await;
                    state
                        .runtimes
                        .insert(runtime.descriptor.runtime_id.clone(), runtime.clone());
                    if let Err(persist_err) = self.persist_state_snapshot(&state) {
                        warn!(
                            error = %persist_err,
                            runtime_id = %runtime.descriptor.runtime_id,
                            "failed to persist teardown retry metadata after manual teardown failure"
                        );
                    }
                }
                statuses.push(AgentControlStatusEnvelope::v1(
                    request_id.clone(),
                    RuntimeLifecyclePhase::Failed,
                    Some(runtime.descriptor.runtime_id.clone()),
                    Some(runtime.descriptor.provider),
                    Some("teardown failed; retry scheduled".to_string()),
                    json!({
                        "next_retry_at": runtime
                            .teardown_retry
                            .as_ref()
                            .and_then(|metadata| metadata.next_retry_at),
                        "attempt_count": runtime
                            .teardown_retry
                            .as_ref()
                            .map(|metadata| metadata.attempt_count)
                            .unwrap_or(0),
                    }),
                ));
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "teardown_failed",
                        Some("manual cleanup may be required".to_string()),
                        Some(format!(
                            "{}; next_retry_at={}",
                            err,
                            runtime
                                .teardown_retry
                                .as_ref()
                                .and_then(|metadata| metadata.next_retry_at)
                                .unwrap_or(0)
                        )),
                    ),
                );
            }
        };

        if teardown_payload_requires_retry(&payload) {
            let attempt_count = previous_attempt_count.saturating_add(1);
            let next_retry_at = now.saturating_add(teardown_backoff_secs(attempt_count));
            runtime.teardown_retry = Some(TeardownRetryMetadata {
                attempt_count,
                last_error: Some(format!(
                    "provider returned teardown={} retryable=true",
                    teardown_payload_state(&payload).unwrap_or("unknown")
                )),
                next_retry_at: Some(next_retry_at),
                last_attempt_at: Some(now),
            });
        } else {
            runtime.teardown_retry = None;
        }
        let mut persist_error: Option<String> = None;
        {
            let mut state = self.state.write().await;
            state
                .runtimes
                .insert(runtime.descriptor.runtime_id.clone(), runtime.clone());
            if let Err(err) = self.persist_state_snapshot(&state) {
                persist_error = Some(format!("{err:#}"));
                warn!(
                    error = %err,
                    runtime_id = %runtime.descriptor.runtime_id,
                    "teardown completed but state persistence failed; continuing with in-memory teardown state"
                );
                statuses.push(AgentControlStatusEnvelope::v1(
                    request_id.clone(),
                    RuntimeLifecyclePhase::Teardown,
                    Some(runtime.descriptor.runtime_id.clone()),
                    Some(runtime.descriptor.provider),
                    Some("teardown completed but state persistence failed".to_string()),
                    Value::Null,
                ));
            }
        }
        if let Some(err) = transition_persist_error {
            if let Value::Object(ref mut map) = payload {
                map.insert("transition_state_persist".to_string(), json!("failed"));
                map.insert("transition_state_persist_error".to_string(), json!(err));
            } else {
                payload = json!({
                    "provider_payload": payload,
                    "transition_state_persist": "failed",
                    "transition_state_persist_error": err,
                });
            }
        }
        if let Some(err) = persist_error {
            if let Value::Object(ref mut map) = payload {
                map.insert("state_persist".to_string(), json!("failed"));
                map.insert("state_persist_error".to_string(), json!(err));
            } else {
                payload = json!({
                    "provider_payload": payload,
                    "state_persist": "failed",
                    "state_persist_error": err,
                });
            }
        }
        if teardown_payload_is_failed(&payload) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "teardown_failed",
                    Some("manual cleanup may be required".to_string()),
                    Some(payload.to_string()),
                ),
            );
        }
        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(request_id, runtime.descriptor_with_lease(), payload),
        )
    }

    async fn handle_get_runtime(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        get_runtime: GetRuntimeCommand,
        statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        let runtime = {
            let state = self.state.read().await;
            state.runtimes.get(&get_runtime.runtime_id).cloned()
        };
        let Some(runtime) = runtime else {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "runtime_not_found",
                    Some("runtime id is unknown to this server".to_string()),
                    Some(get_runtime.runtime_id),
                ),
            );
        };
        if !self.can_access_runtime(requester_pubkey_hex, &runtime) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "runtime_not_found",
                    Some("runtime id is unknown to this server".to_string()),
                    Some(get_runtime.runtime_id),
                ),
            );
        }
        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(
                request_id,
                runtime.descriptor_with_lease(),
                json!({"operation":"get_runtime"}),
            ),
        )
    }

    async fn handle_list_runtimes(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        list: ListRuntimesCommand,
        statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        let runtimes: Vec<RuntimeDescriptor> = {
            let state = self.state.read().await;
            state
                .runtimes
                .values()
                .filter(|runtime| self.can_access_runtime(requester_pubkey_hex, runtime))
                .map(|runtime| runtime.descriptor_with_lease())
                .collect()
        };
        let mut filtered: Vec<RuntimeDescriptor> = runtimes
            .into_iter()
            .filter(|descriptor| {
                if let Some(provider) = list.provider {
                    if descriptor.provider != provider {
                        return false;
                    }
                }
                if let Some(protocol) = list.protocol {
                    if !descriptor.protocol_compatibility.contains(&protocol) {
                        return false;
                    }
                }
                if let Some(phase) = list.lifecycle_phase {
                    if descriptor.lifecycle_phase != phase {
                        return false;
                    }
                }
                if let Some(requested_class) = list.runtime_class.as_deref() {
                    if descriptor.runtime_class.as_deref() != Some(requested_class) {
                        return false;
                    }
                }
                true
            })
            .collect();
        filtered.sort_by(|a, b| a.runtime_id.cmp(&b.runtime_id));
        if let Some(limit) = list.limit {
            filtered.truncate(limit);
        }
        let summary = filtered
            .first()
            .cloned()
            .unwrap_or_else(default_list_summary_descriptor);
        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(
                request_id,
                summary,
                json!({
                    "operation":"list_runtimes",
                    "count": filtered.len(),
                    "runtimes": filtered,
                }),
            ),
        )
    }

    async fn handle_get_capabilities(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        _get_capabilities: GetCapabilitiesCommand,
        statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        if !self.provision_policy.is_allowed(requester_pubkey_hex) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "provision_unauthorized",
                    Some(
                        "set PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST to include this requester pubkey"
                            .to_string(),
                    ),
                    Some(format!("requester_pubkey={requester_pubkey_hex}")),
                ),
            );
        }
        let payload = json!({
            "operation": "get_capabilities",
            "providers": [
                {
                    "provider": provider_name(ProviderKind::Fly),
                    "runtime_class": runtime_profile(ProviderKind::Fly).runtime_class,
                    "region": runtime_profile(ProviderKind::Fly).region,
                    "workload_kinds": ["oci"],
                    "build_support": ["prebuilt_artifact", "build_ref"],
                },
                {
                    "provider": provider_name(ProviderKind::Microvm),
                    "runtime_class": runtime_profile(ProviderKind::Microvm).runtime_class,
                    "region": runtime_profile(ProviderKind::Microvm).region,
                    "workload_kinds": ["nix"],
                    "build_support": ["prebuilt_artifact", "build_ref"],
                }
            ],
            "limits": {
                "ttl_sec_min": 60,
                "ttl_sec_default": runtime_ttl_secs(),
                "build_max_active": self.build_policy.max_active_builds,
                "build_max_submissions_per_hour": self.build_policy.max_submissions_per_hour,
                "build_max_context_bytes": self.build_policy.max_context_bytes,
            },
            "policy_flags": {
                "distribution_enabled": true,
                "build_enabled": true,
                "advanced_workload_enabled": self.v2_flags.advanced_workload_enabled,
            }
        });

        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(request_id, default_list_summary_descriptor(), payload),
        )
    }

    async fn handle_resolve_distribution(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        resolve_distribution: ResolveDistributionCommand,
        statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        if !self.provision_policy.is_allowed(requester_pubkey_hex) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "provision_unauthorized",
                    Some(
                        "set PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST to include this requester pubkey"
                            .to_string(),
                    ),
                    Some(format!("requester_pubkey={requester_pubkey_hex}")),
                ),
            );
        }
        let manifests = load_distribution_manifests();
        let Some(manifest) = manifests.get(&resolve_distribution.distribution_ref) else {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "distribution_not_found",
                    Some("distribution_ref is unknown on this server".to_string()),
                    Some(resolve_distribution.distribution_ref),
                ),
            );
        };
        let Some(preset) = manifest.presets.get(&resolve_distribution.preset) else {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "distribution_preset_not_found",
                    Some("preset is not available for this distribution".to_string()),
                    Some(resolve_distribution.preset),
                ),
            );
        };

        let mut runtime_class = preset.runtime_class.clone();
        let mut ttl_sec = preset.ttl_sec.max(60);
        let mut region_hint = preset.region_hint.clone();
        let mut artifact_ref = preset.artifact_ref.clone();
        let mut applied_overrides = serde_json::Map::new();
        let mut allowed_override_keys: HashSet<String> =
            manifest.allowed_override_keys.iter().cloned().collect();
        for key in &preset.allowed_override_keys {
            allowed_override_keys.insert(key.clone());
        }

        if let Some(overrides_json) = resolve_distribution.overrides_json.as_deref() {
            let overrides_value: Value = match serde_json::from_str(overrides_json) {
                Ok(parsed) => parsed,
                Err(err) => {
                    return CommandOutcome::error(
                        statuses,
                        AgentControlErrorEnvelope::v1(
                            request_id,
                            "distribution_overrides_invalid_json",
                            Some("overrides_json must be valid JSON".to_string()),
                            Some(err.to_string()),
                        ),
                    );
                }
            };
            let Some(overrides_obj) = overrides_value.as_object() else {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "distribution_overrides_invalid_shape",
                        Some("overrides_json must decode to a JSON object".to_string()),
                        None,
                    ),
                );
            };
            for key in overrides_obj.keys() {
                if !allowed_override_keys.contains(key) {
                    return CommandOutcome::error(
                        statuses,
                        AgentControlErrorEnvelope::v1(
                            request_id,
                            "distribution_override_not_allowed",
                            Some("override key is not allowed by distribution policy".to_string()),
                            Some(key.clone()),
                        ),
                    );
                }
            }
            if let Some(value) = overrides_obj.get("ttl_sec").and_then(Value::as_u64) {
                ttl_sec = value.max(60);
                applied_overrides.insert("ttl_sec".to_string(), json!(ttl_sec));
            }
            if let Some(value) = overrides_obj.get("region_hint").and_then(Value::as_str) {
                let region = value.trim();
                if !region.is_empty() {
                    region_hint = Some(region.to_string());
                    applied_overrides.insert("region_hint".to_string(), json!(region_hint));
                }
            }
            if let Some(value) = overrides_obj.get("runtime_class").and_then(Value::as_str) {
                let class = value.trim();
                if !class.is_empty() {
                    runtime_class = Some(class.to_string());
                    applied_overrides.insert("runtime_class".to_string(), json!(runtime_class));
                }
            }
            if let Some(value) = overrides_obj.get("artifact_ref").and_then(Value::as_str) {
                let normalized = value.trim();
                if !normalized.is_empty() {
                    if let Some(kind) = preset.build_kind {
                        if let Err(err) = ensure_immutable_artifact_ref(kind, normalized) {
                            return CommandOutcome::error(
                                statuses,
                                AgentControlErrorEnvelope::v1(
                                    request_id,
                                    "distribution_artifact_invalid",
                                    Some("artifact_ref must be immutable".to_string()),
                                    Some(err.to_string()),
                                ),
                            );
                        }
                    }
                    artifact_ref = Some(normalized.to_string());
                    applied_overrides.insert("artifact_ref".to_string(), json!(artifact_ref));
                }
            }
        }

        let mut descriptor = default_list_summary_descriptor();
        descriptor.provider = preset.provider;
        descriptor.runtime_class = runtime_class.clone();
        descriptor.region = region_hint.clone();
        descriptor.protocol_compatibility = vec![ProtocolKind::Acp];
        descriptor.metadata = json!({
            "resolved_distribution": {
                "distribution_ref": manifest.distribution_ref,
                "preset": resolve_distribution.preset,
                "provider": provider_name(preset.provider),
                "runtime_class": runtime_class,
                "ttl_sec": ttl_sec,
                "region_hint": region_hint,
                "build_kind": preset.build_kind.map(build_kind_name),
                "artifact_ref": artifact_ref.clone(),
                "overrides": Value::Object(applied_overrides),
            }
        });

        let payload = json!({
            "operation": "resolve_distribution",
            "distribution_ref": manifest.distribution_ref,
            "preset": resolve_distribution.preset,
            "resolved_provision": {
                "provider": provider_name(preset.provider),
                "protocol": protocol_name(ProtocolKind::Acp),
                "runtime_class": descriptor.runtime_class,
                "requested_ttl_sec": ttl_sec,
                "region_hint": descriptor.region,
                "build_kind": preset.build_kind.map(build_kind_name),
                "artifact_ref": artifact_ref,
            },
            "policy": {
                "build_enabled": true,
                "advanced_workload_enabled": self.v2_flags.advanced_workload_enabled,
            }
        });

        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(request_id, descriptor, payload),
        )
    }

    async fn handle_submit_build(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        submit_build: SubmitBuildCommand,
        statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        if !self.provision_policy.is_allowed(requester_pubkey_hex) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "provision_unauthorized",
                    Some(
                        "set PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST to include this requester pubkey"
                            .to_string(),
                    ),
                    Some(format!("requester_pubkey={requester_pubkey_hex}")),
                ),
            );
        }
        let now = unix_now_secs();
        let normalized = match normalize_submit_build_command(
            &submit_build,
            &self.build_policy,
            self.v2_flags.advanced_workload_enabled,
        ) {
            Ok(normalized) => normalized,
            Err((code, hint, detail)) => {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(request_id, code, Some(hint), Some(detail)),
                );
            }
        };
        let build_id = new_build_id();
        let mut build = BuildRecord {
            build_id: build_id.clone(),
            owner_pubkey_hex: requester_pubkey_hex.to_string(),
            build_kind: normalized.build_kind,
            phase: BuildPhase::Queued,
            source_ref: normalized.source_ref.clone(),
            artifact_ref: normalized.artifact_ref.clone(),
            created_at: now,
            updated_at: now,
            deadline_at: now.saturating_add(normalized.timeout_sec),
            ready_at: None,
            context_bytes: normalized.context_bytes,
            timeout_sec: normalized.timeout_sec,
            error_code: None,
            error_detail: None,
            canceled_at: None,
        };

        {
            let mut state = self.state.write().await;
            let active = state
                .builds
                .values()
                .filter(|existing| !existing.phase.is_terminal())
                .count();
            if active >= self.build_policy.max_active_builds {
                self.push_audit_event(
                    &mut state,
                    requester_pubkey_hex,
                    "submit_build",
                    "denied_quota_active",
                    json!({
                        "max_active_builds": self.build_policy.max_active_builds,
                        "active_builds": active,
                    }),
                );
                if let Err(err) = self.persist_state_snapshot(&state) {
                    warn!(error = %err, "failed to persist denied_quota_active audit event");
                }
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_quota_exceeded",
                        Some("too many active builds on this server".to_string()),
                        Some(format!(
                            "active_builds={active}, max_active_builds={}",
                            self.build_policy.max_active_builds
                        )),
                    ),
                );
            }
            let recent_submissions = state
                .builds
                .values()
                .filter(|existing| {
                    existing.owner_pubkey_hex == requester_pubkey_hex
                        && existing.created_at >= now.saturating_sub(3600)
                })
                .count();
            if recent_submissions >= self.build_policy.max_submissions_per_hour {
                self.push_audit_event(
                    &mut state,
                    requester_pubkey_hex,
                    "submit_build",
                    "denied_rate_limit",
                    json!({
                        "max_submissions_per_hour": self.build_policy.max_submissions_per_hour,
                        "recent_submissions": recent_submissions,
                    }),
                );
                if let Err(err) = self.persist_state_snapshot(&state) {
                    warn!(error = %err, "failed to persist denied_rate_limit audit event");
                }
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_rate_limited",
                        Some("build submission quota exceeded for this requester".to_string()),
                        Some(format!(
                            "recent_submissions={recent_submissions}, max_submissions_per_hour={}",
                            self.build_policy.max_submissions_per_hour
                        )),
                    ),
                );
            }
            state.builds.insert(build_id.clone(), build.clone());
            self.push_audit_event(
                &mut state,
                requester_pubkey_hex,
                "submit_build",
                "accepted",
                json!({
                    "build_id": build_id.clone(),
                    "build_kind": build_kind_name(build.build_kind),
                    "source_ref": build.source_ref.clone(),
                    "artifact_ref": build.artifact_ref.clone(),
                }),
            );
            if let Err(err) = self.persist_state_snapshot(&state) {
                state.builds.remove(&build_id);
                self.push_audit_event(
                    &mut state,
                    requester_pubkey_hex,
                    "submit_build",
                    "state_persist_failed",
                    json!({
                        "build_id": build_id.clone(),
                        "error": format!("{err:#}"),
                    }),
                );
                let _ = self.persist_state_snapshot(&state);
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "state_persist_failed",
                        Some("failed to persist build submission".to_string()),
                        Some(format!("{err:#}")),
                    ),
                );
            }
        }

        let mut artifact_to_store: Option<ArtifactRecord> = None;
        match self.build_service.submit(&build_id, &normalized, now).await {
            Ok(BuildSubmitOutcome::ImmediateSuccess { artifact_ref }) => {
                build.phase = BuildPhase::PublishingArtifact;
                build.updated_at = now;
                build.phase = BuildPhase::Succeeded;
                build.artifact_ref = Some(artifact_ref.clone());
                artifact_to_store = Some(ArtifactRecord {
                    artifact_ref,
                    build_kind: build.build_kind,
                    owner_pubkey_hex: requester_pubkey_hex.to_string(),
                    created_at: now,
                    last_used_at: now,
                    expires_at: now.saturating_add(self.build_policy.artifact_ttl_secs),
                    source_build_id: Some(build_id.clone()),
                });
            }
            Ok(BuildSubmitOutcome::Pending {
                next_phase,
                ready_at,
            }) => {
                build.phase = BuildPhase::Validating;
                build.updated_at = now;
                build.phase = next_phase;
                build.ready_at = Some(ready_at);
            }
            Err(err) => {
                build.phase = BuildPhase::Failed;
                build.updated_at = now;
                build.error_code = Some("build_submit_failed".to_string());
                build.error_detail = Some(err.to_string());
            }
        }

        {
            let mut state = self.state.write().await;
            state.builds.insert(build_id.clone(), build.clone());
            if let Some(artifact) = artifact_to_store {
                state
                    .artifacts
                    .insert(artifact.artifact_ref.clone(), artifact);
            }
            self.push_audit_event(
                &mut state,
                requester_pubkey_hex,
                "submit_build",
                if build.phase == BuildPhase::Failed {
                    "failed"
                } else {
                    "ok"
                },
                json!({
                    "build_id": build_id.clone(),
                    "phase": build.phase,
                    "artifact_ref": build.artifact_ref.clone(),
                }),
            );
            if let Err(err) = self.persist_state_snapshot(&state) {
                warn!(
                    error = %err,
                    build_id = %build.build_id,
                    "failed to persist build terminal/update state"
                );
            }
        }

        if build.phase == BuildPhase::Failed {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    build
                        .error_code
                        .clone()
                        .unwrap_or_else(|| "build_submit_failed".to_string()),
                    Some("build submission was rejected".to_string()),
                    build.error_detail.clone(),
                ),
            );
        }

        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(
                request_id,
                default_list_summary_descriptor(),
                json!({
                    "operation": "submit_build",
                    "build": build,
                }),
            ),
        )
    }

    async fn handle_get_build(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        get_build: GetBuildCommand,
        statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        let build_id = get_build.build_id;
        if !self.provision_policy.is_allowed(requester_pubkey_hex) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "provision_unauthorized",
                    Some(
                        "set PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST to include this requester pubkey"
                            .to_string(),
                    ),
                    Some(format!("requester_pubkey={requester_pubkey_hex}")),
                ),
            );
        }
        let pre_refresh = {
            let state = self.state.read().await;
            state.builds.get(&build_id).cloned()
        };
        let Some(pre_refresh) = pre_refresh else {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "build_not_found",
                    Some("build id is unknown to this server".to_string()),
                    Some(build_id.clone()),
                ),
            );
        };
        if pre_refresh.owner_pubkey_hex != requester_pubkey_hex {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "build_not_found",
                    Some("build id is unknown to this server".to_string()),
                    Some(build_id.clone()),
                ),
            );
        }

        let build = match self
            .refresh_build_progress(&pre_refresh.build_id, unix_now_secs())
            .await
        {
            Ok(Some(build)) => build,
            Ok(None) => {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_not_found",
                        Some("build id is unknown to this server".to_string()),
                        Some(build_id.clone()),
                    ),
                );
            }
            Err(err) => {
                return CommandOutcome::error(
                    statuses,
                    AgentControlErrorEnvelope::v1(
                        request_id,
                        "build_refresh_failed",
                        Some("failed to refresh build state".to_string()),
                        Some(err.to_string()),
                    ),
                );
            }
        };
        if build.owner_pubkey_hex != requester_pubkey_hex {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "build_not_found",
                    Some("build id is unknown to this server".to_string()),
                    Some(build_id),
                ),
            );
        }
        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(
                request_id,
                default_list_summary_descriptor(),
                json!({
                    "operation": "get_build",
                    "build": build,
                }),
            ),
        )
    }

    async fn handle_cancel_build(
        &self,
        requester_pubkey_hex: &str,
        request_id: String,
        cancel_build: CancelBuildCommand,
        statuses: Vec<AgentControlStatusEnvelope>,
    ) -> CommandOutcome {
        if !self.provision_policy.is_allowed(requester_pubkey_hex) {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "provision_unauthorized",
                    Some(
                        "set PIKA_AGENT_CONTROL_PROVISION_ALLOWLIST to include this requester pubkey"
                            .to_string(),
                    ),
                    Some(format!("requester_pubkey={requester_pubkey_hex}")),
                ),
            );
        }
        let now = unix_now_secs();
        let build = {
            let state = self.state.read().await;
            state.builds.get(&cancel_build.build_id).cloned()
        };
        let Some(mut build) = build else {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "build_not_found",
                    Some("build id is unknown to this server".to_string()),
                    Some(cancel_build.build_id),
                ),
            );
        };
        if build.owner_pubkey_hex != requester_pubkey_hex {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "build_not_found",
                    Some("build id is unknown to this server".to_string()),
                    Some(cancel_build.build_id),
                ),
            );
        }

        if build.phase.is_terminal() {
            return CommandOutcome::result(
                statuses,
                AgentControlResultEnvelope::v1(
                    request_id,
                    default_list_summary_descriptor(),
                    json!({
                        "operation": "cancel_build",
                        "build": build,
                        "already_terminal": true,
                    }),
                ),
            );
        }

        if let Err(err) = self.build_service.cancel(&build, now).await {
            return CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    "build_cancel_failed",
                    Some("build service failed to cancel build".to_string()),
                    Some(err.to_string()),
                ),
            );
        }
        build.phase = BuildPhase::Canceled;
        build.updated_at = now;
        build.canceled_at = Some(now);
        build.error_code = Some("build_canceled".to_string());
        build.error_detail = Some("build canceled by requester".to_string());

        {
            let mut state = self.state.write().await;
            state.builds.insert(build.build_id.clone(), build.clone());
            self.push_audit_event(
                &mut state,
                requester_pubkey_hex,
                "cancel_build",
                "ok",
                json!({
                    "build_id": build.build_id,
                }),
            );
            if let Err(err) = self.persist_state_snapshot(&state) {
                warn!(
                    error = %err,
                    build_id = %build.build_id,
                    "failed to persist canceled build state"
                );
            }
        }

        CommandOutcome::result(
            statuses,
            AgentControlResultEnvelope::v1(
                request_id,
                default_list_summary_descriptor(),
                json!({
                    "operation": "cancel_build",
                    "build": build,
                    "already_terminal": false,
                }),
            ),
        )
    }

    async fn refresh_build_progress(
        &self,
        build_id: &str,
        now: u64,
    ) -> anyhow::Result<Option<BuildRecord>> {
        let current = {
            let state = self.state.read().await;
            state.builds.get(build_id).cloned()
        };
        let Some(mut build) = current else {
            return Ok(None);
        };

        let poll_outcome = self
            .build_service
            .poll(&build, now, &self.build_policy.allowed_source_rules)
            .await?;

        let mut changed = false;
        if let Some(outcome) = poll_outcome {
            if build.phase != outcome.phase {
                build.phase = outcome.phase;
                changed = true;
            }
            if outcome.artifact_ref.is_some() && build.artifact_ref != outcome.artifact_ref {
                build.artifact_ref = outcome.artifact_ref;
                changed = true;
            }
            if build.error_code != outcome.error_code {
                build.error_code = outcome.error_code;
                changed = true;
            }
            if build.error_detail != outcome.error_detail {
                build.error_detail = outcome.error_detail;
                changed = true;
            }
        }
        if now >= build.deadline_at && !build.phase.is_terminal() {
            build.phase = BuildPhase::Failed;
            build.error_code = Some("build_timeout".to_string());
            build.error_detail = Some("build exceeded timeout".to_string());
            changed = true;
        }
        if !changed {
            return Ok(Some(build));
        }

        build.updated_at = now;
        let artifact_to_store = if build.phase == BuildPhase::Succeeded {
            build
                .artifact_ref
                .as_ref()
                .map(|artifact_ref| ArtifactRecord {
                    artifact_ref: artifact_ref.clone(),
                    build_kind: build.build_kind,
                    owner_pubkey_hex: build.owner_pubkey_hex.clone(),
                    created_at: now,
                    last_used_at: now,
                    expires_at: now.saturating_add(self.build_policy.artifact_ttl_secs),
                    source_build_id: Some(build.build_id.clone()),
                })
        } else {
            None
        };

        let mut state = self.state.write().await;
        state.builds.insert(build.build_id.clone(), build.clone());
        if let Some(artifact) = artifact_to_store {
            state
                .artifacts
                .insert(artifact.artifact_ref.clone(), artifact);
        }
        if let Err(err) = self.persist_state_snapshot(&state) {
            warn!(
                error = %err,
                build_id = %build.build_id,
                "failed to persist refreshed build state"
            );
        }
        Ok(Some(build))
    }

    async fn reap_expired_runtimes_once(&self) -> anyhow::Result<()> {
        let now = unix_now_secs();
        self.reap_expired_runtimes_at(now).await?;
        let _ = self.reconcile_pending_builds_at(now).await?;
        let _ = self.garbage_collect_artifacts_at(now).await?;
        Ok(())
    }

    async fn reconcile_pending_builds_at(&self, now: u64) -> anyhow::Result<usize> {
        let pending_ids: Vec<String> = {
            let state = self.state.read().await;
            state
                .builds
                .iter()
                .filter(|(_, build)| !build.phase.is_terminal())
                .map(|(build_id, _)| build_id.clone())
                .collect()
        };
        for build_id in &pending_ids {
            let _ = self.refresh_build_progress(build_id, now).await?;
        }
        Ok(pending_ids.len())
    }

    async fn garbage_collect_artifacts_at(&self, now: u64) -> anyhow::Result<usize> {
        let mut state = self.state.write().await;
        let mut in_use = HashSet::new();
        for runtime in state.runtimes.values() {
            if runtime.descriptor.lifecycle_phase == RuntimeLifecyclePhase::Teardown {
                continue;
            }
            if let Some(artifact_ref) = runtime
                .descriptor
                .metadata
                .get("artifact_ref")
                .and_then(Value::as_str)
            {
                in_use.insert(artifact_ref.to_string());
            }
        }
        let before = state.artifacts.len();
        state.artifacts.retain(|artifact_ref, artifact| {
            if in_use.contains(artifact_ref) {
                return true;
            }
            artifact.expires_at > now
        });
        let removed = before.saturating_sub(state.artifacts.len());
        if removed > 0 {
            self.push_audit_event(
                &mut state,
                "system",
                "artifact_gc",
                "ok",
                json!({
                    "removed": removed,
                }),
            );
            if let Err(err) = self.persist_state_snapshot(&state) {
                warn!(error = %err, "failed to persist artifact gc state");
            }
        }
        Ok(removed)
    }

    async fn reap_expired_runtimes_at(&self, now: u64) -> anyhow::Result<usize> {
        let runtime_ids: Vec<String> = {
            let state = self.state.read().await;
            state
                .runtimes
                .iter()
                .filter(|(_, runtime)| runtime_due_for_reaper(runtime, now))
                .map(|(runtime_id, _)| runtime_id.clone())
                .collect()
        };

        for runtime_id in &runtime_ids {
            let runtime = {
                let state = self.state.read().await;
                state.runtimes.get(runtime_id).cloned()
            };
            let Some(mut runtime) = runtime else {
                continue;
            };
            let previous_attempt_count = runtime
                .teardown_retry
                .as_ref()
                .map(|metadata| metadata.attempt_count)
                .unwrap_or(0);
            runtime.descriptor.lifecycle_phase = RuntimeLifecyclePhase::Teardown;

            {
                let mut state = self.state.write().await;
                state
                    .runtimes
                    .insert(runtime.descriptor.runtime_id.clone(), runtime.clone());
                if let Err(err) = self.persist_state_snapshot(&state) {
                    runtime.teardown_retry = Some(TeardownRetryMetadata {
                        attempt_count: previous_attempt_count,
                        last_error: Some(format!("persist teardown transition failed: {err:#}")),
                        next_retry_at: Some(now),
                        last_attempt_at: None,
                    });
                    state
                        .runtimes
                        .insert(runtime.descriptor.runtime_id.clone(), runtime.clone());
                    warn!(
                        error = %err,
                        runtime_id = %runtime.descriptor.runtime_id,
                        "failed to persist reaper teardown transition; runtime marked retryable in-memory"
                    );
                    continue;
                }
            }

            let adapter = self.adapter_for(runtime.descriptor.provider);
            match adapter.teardown(&runtime).await {
                Ok(payload) => {
                    if teardown_payload_requires_retry(&payload) {
                        let attempt_count = previous_attempt_count.saturating_add(1);
                        let backoff = teardown_backoff_secs(attempt_count);
                        runtime.teardown_retry = Some(TeardownRetryMetadata {
                            attempt_count,
                            last_error: Some(format!(
                                "provider returned teardown={} retryable=true",
                                teardown_payload_state(&payload).unwrap_or("unknown")
                            )),
                            next_retry_at: Some(now.saturating_add(backoff)),
                            last_attempt_at: Some(now),
                        });
                    } else {
                        runtime.teardown_retry = None;
                    }
                    let mut state = self.state.write().await;
                    state
                        .runtimes
                        .insert(runtime.descriptor.runtime_id.clone(), runtime.clone());
                    if let Err(err) = self.persist_state_snapshot(&state) {
                        warn!(
                            error = %err,
                            runtime_id = %runtime.descriptor.runtime_id,
                            "failed to persist reaper teardown success state"
                        );
                    }
                    if teardown_payload_is_failed(&payload) {
                        warn!(
                            runtime_id = %runtime.descriptor.runtime_id,
                            payload = %payload,
                            "reaper provider teardown returned terminal failure payload"
                        );
                    }
                }
                Err(err) => {
                    let attempt_count = previous_attempt_count.saturating_add(1);
                    let backoff = teardown_backoff_secs(attempt_count);
                    let next_retry_at = now.saturating_add(backoff);
                    runtime.teardown_retry = Some(TeardownRetryMetadata {
                        attempt_count,
                        last_error: Some(format!("{err:#}")),
                        next_retry_at: Some(next_retry_at),
                        last_attempt_at: Some(now),
                    });
                    let mut state = self.state.write().await;
                    state
                        .runtimes
                        .insert(runtime.descriptor.runtime_id.clone(), runtime.clone());
                    if let Err(persist_err) = self.persist_state_snapshot(&state) {
                        warn!(
                            error = %persist_err,
                            runtime_id = %runtime.descriptor.runtime_id,
                            "failed to persist reaper retry metadata"
                        );
                    }
                    warn!(
                        error = %err,
                        runtime_id = %runtime.descriptor.runtime_id,
                        next_retry_at,
                        attempt_count,
                        "reaper teardown failed; retry scheduled"
                    );
                }
            }
        }

        Ok(runtime_ids.len())
    }

    #[cfg(test)]
    async fn reap_expired_runtimes_for_test(&self, now: u64) -> anyhow::Result<usize> {
        self.reap_expired_runtimes_at(now).await
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RuntimeRecord {
    #[serde(default)]
    owner_pubkey_hex: String,
    descriptor: RuntimeDescriptor,
    provider_handle: ProviderHandle,
    #[serde(default)]
    created_at: u64,
    #[serde(default)]
    expires_at: u64,
    #[serde(default)]
    teardown_retry: Option<TeardownRetryMetadata>,
}

impl RuntimeRecord {
    fn descriptor_with_lease(&self) -> RuntimeDescriptor {
        let mut descriptor = self.descriptor.clone();
        descriptor.metadata = with_lease_metadata(
            descriptor.metadata,
            self.created_at,
            self.expires_at,
            self.teardown_retry.as_ref(),
        );
        descriptor
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct TeardownRetryMetadata {
    #[serde(default)]
    attempt_count: u32,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    next_retry_at: Option<u64>,
    #[serde(default)]
    last_attempt_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum ProviderHandle {
    Fly {
        machine_id: String,
        volume_id: String,
        app_name: String,
    },
    Microvm {
        vm_id: String,
        spawner_url: String,
        keep: bool,
    },
}

#[derive(Default)]
struct ControlState {
    runtimes: HashMap<String, RuntimeRecord>,
    builds: HashMap<String, BuildRecord>,
    artifacts: HashMap<String, ArtifactRecord>,
    audit_log: VecDeque<AuditEvent>,
    idempotency: HashMap<(String, String), CachedTerminal>,
    idempotency_order: VecDeque<(String, String)>,
}

impl ControlState {
    fn insert_idempotency(
        &mut self,
        key: (String, String),
        terminal: CachedTerminal,
        max_entries: usize,
    ) {
        if self.idempotency.contains_key(&key) {
            self.idempotency_order.retain(|existing| existing != &key);
        }
        self.idempotency.insert(key.clone(), terminal);
        self.idempotency_order.push_back(key);
        while self.idempotency_order.len() > max_entries {
            if let Some(oldest) = self.idempotency_order.pop_front() {
                self.idempotency.remove(&oldest);
            }
        }
    }

    fn truncate_idempotency(&mut self, max_entries: usize) {
        while self.idempotency_order.len() > max_entries {
            if let Some(oldest) = self.idempotency_order.pop_front() {
                self.idempotency.remove(&oldest);
            }
        }
    }

    fn ensure_runtime_lease_fields(&mut self, now: u64, ttl_secs: u64) -> usize {
        let mut migrated = 0usize;
        for runtime in self.runtimes.values_mut() {
            if runtime.created_at == 0 {
                runtime.created_at = now;
                migrated += 1;
            }
            if runtime.expires_at == 0 || runtime.expires_at < runtime.created_at {
                runtime.expires_at = runtime.created_at.saturating_add(ttl_secs);
                migrated += 1;
            }
        }
        migrated
    }

    fn ensure_v2_build_fields(&mut self, now: u64, artifact_ttl_secs: u64) -> usize {
        let mut migrated = 0usize;
        for build in self.builds.values_mut() {
            if build.updated_at == 0 {
                build.updated_at = build.created_at.max(now);
                migrated += 1;
            }
            if build.timeout_sec == 0 {
                build.timeout_sec = DEFAULT_BUILD_TIMEOUT_SECS;
                migrated += 1;
            }
            if build.deadline_at == 0 {
                build.deadline_at = build
                    .created_at
                    .max(now)
                    .saturating_add(build.timeout_sec.max(60));
                migrated += 1;
            }
        }
        for artifact in self.artifacts.values_mut() {
            if artifact.last_used_at == 0 {
                artifact.last_used_at = artifact.created_at.max(now);
                migrated += 1;
            }
            if artifact.expires_at == 0 {
                artifact.expires_at = artifact
                    .created_at
                    .max(now)
                    .saturating_add(artifact_ttl_secs.max(60));
                migrated += 1;
            }
        }
        migrated
    }
}

#[derive(Default, Serialize, Deserialize)]
struct PersistedControlState {
    #[serde(default)]
    runtimes: HashMap<String, RuntimeRecord>,
    #[serde(default)]
    builds: HashMap<String, BuildRecord>,
    #[serde(default)]
    artifacts: HashMap<String, ArtifactRecord>,
    #[serde(default)]
    audit_log: VecDeque<AuditEvent>,
    #[serde(default)]
    idempotency: Vec<PersistedIdempotencyEntry>,
}

#[derive(Serialize, Deserialize)]
struct PersistedIdempotencyEntry {
    requester_pubkey_hex: String,
    idempotency_key: String,
    terminal: CachedTerminal,
}

impl From<PersistedControlState> for ControlState {
    fn from(value: PersistedControlState) -> Self {
        let mut state = Self {
            runtimes: value.runtimes,
            builds: value.builds,
            artifacts: value.artifacts,
            audit_log: value.audit_log,
            ..Self::default()
        };
        for entry in value.idempotency {
            state.insert_idempotency(
                (entry.requester_pubkey_hex, entry.idempotency_key),
                entry.terminal,
                usize::MAX,
            );
        }
        state
    }
}

impl From<&ControlState> for PersistedControlState {
    fn from(value: &ControlState) -> Self {
        let mut idempotency = Vec::new();
        for (requester_pubkey_hex, idempotency_key) in &value.idempotency_order {
            let Some(terminal) = value
                .idempotency
                .get(&(requester_pubkey_hex.clone(), idempotency_key.clone()))
            else {
                continue;
            };
            idempotency.push(PersistedIdempotencyEntry {
                requester_pubkey_hex: requester_pubkey_hex.clone(),
                idempotency_key: idempotency_key.clone(),
                terminal: terminal.clone(),
            });
        }
        Self {
            runtimes: value.runtimes.clone(),
            builds: value.builds.clone(),
            artifacts: value.artifacts.clone(),
            audit_log: value.audit_log.clone(),
            idempotency,
        }
    }
}

#[derive(Clone, Debug)]
struct ControlStatePersistence {
    path: PathBuf,
}

impl ControlStatePersistence {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn load(&self) -> anyhow::Result<ControlState> {
        if !self.path.exists() {
            return Ok(ControlState::default());
        }
        let data = std::fs::read_to_string(&self.path)
            .with_context(|| format!("read control state {}", self.path.display()))?;
        if data.trim().is_empty() {
            return Ok(ControlState::default());
        }
        let mut raw: Value = match serde_json::from_str(&data) {
            Ok(raw) => raw,
            Err(err) => {
                warn!(
                    error = %err,
                    path = %self.path.display(),
                    "failed to decode control state; starting with empty state"
                );
                return Ok(ControlState::default());
            }
        };
        let migrated_protocol_values = migrate_legacy_protocol_values(&mut raw);
        if migrated_protocol_values > 0 {
            warn!(
                count = migrated_protocol_values,
                path = %self.path.display(),
                "migrated legacy protocol values to acp while loading control state"
            );
        }
        let persisted: PersistedControlState = match serde_json::from_value(raw) {
            Ok(persisted) => persisted,
            Err(err) => {
                warn!(
                    error = %err,
                    path = %self.path.display(),
                    "failed to decode control state; starting with empty state"
                );
                return Ok(ControlState::default());
            }
        };
        let state: ControlState = persisted.into();
        let legacy_ownerless = state
            .runtimes
            .values()
            .filter(|runtime| runtime.owner_pubkey_hex.is_empty())
            .count();
        if legacy_ownerless > 0 {
            warn!(
                count = legacy_ownerless,
                path = %self.path.display(),
                "loaded legacy runtimes without owner pubkeys; access is limited to provisioning-allowed requesters"
            );
        }
        Ok(state)
    }

    fn save(&self, state: &ControlState) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!("create control state directory {}", parent.display())
                })?;
            }
        }
        let persisted = PersistedControlState::from(state);
        let serialized =
            serde_json::to_string_pretty(&persisted).context("encode control state json")?;
        let tmp_path = self.path.with_extension("tmp");
        std::fs::write(&tmp_path, serialized)
            .with_context(|| format!("write control state {}", tmp_path.display()))?;
        std::fs::rename(&tmp_path, &self.path)
            .with_context(|| format!("persist control state {}", self.path.display()))?;
        Ok(())
    }
}

fn migrate_legacy_protocol_values(root: &mut Value) -> usize {
    let Some(runtimes) = root.get_mut("runtimes").and_then(Value::as_object_mut) else {
        return 0;
    };
    let mut migrated = 0usize;
    for runtime in runtimes.values_mut() {
        let Some(descriptor) = runtime.get_mut("descriptor").and_then(Value::as_object_mut) else {
            continue;
        };
        migrated += migrate_legacy_descriptor_protocols(descriptor);
    }
    migrated
}

fn migrate_legacy_descriptor_protocols(descriptor: &mut serde_json::Map<String, Value>) -> usize {
    let mut migrated = 0usize;
    if let Some(protocols) = descriptor
        .get_mut("protocol_compatibility")
        .and_then(Value::as_array_mut)
    {
        for protocol in protocols.iter_mut() {
            let Some(raw) = protocol.as_str() else {
                continue;
            };
            let Some(normalized) = normalize_legacy_protocol_name(raw) else {
                continue;
            };
            if raw != normalized {
                *protocol = Value::String(normalized.to_string());
                migrated += 1;
            }
        }
        return migrated;
    }

    let Some(raw) = descriptor
        .get("protocol")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return migrated;
    };
    let Some(normalized) = normalize_legacy_protocol_name(&raw) else {
        return migrated;
    };
    descriptor.insert(
        "protocol_compatibility".to_string(),
        Value::Array(vec![Value::String(normalized.to_string())]),
    );
    if raw != normalized {
        migrated += 1;
    }
    migrated
}

fn normalize_legacy_protocol_name(raw: &str) -> Option<&'static str> {
    if raw.eq_ignore_ascii_case("acp") || raw.eq_ignore_ascii_case("pi") {
        Some("acp")
    } else {
        None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum CachedTerminal {
    Result {
        provider: ProviderKind,
        runtime_id: String,
        runtime: Box<RuntimeDescriptor>,
        payload: Value,
    },
    Error {
        provider: Option<ProviderKind>,
        code: String,
        hint: Option<String>,
        detail: Option<String>,
    },
}

impl CachedTerminal {
    fn provider(&self) -> Option<ProviderKind> {
        match self {
            Self::Result { provider, .. } => Some(*provider),
            Self::Error { provider, .. } => *provider,
        }
    }

    fn runtime_id(&self) -> Option<String> {
        match self {
            Self::Result { runtime_id, .. } => Some(runtime_id.clone()),
            Self::Error { .. } => None,
        }
    }

    fn to_outcome(
        &self,
        statuses: Vec<AgentControlStatusEnvelope>,
        request_id: String,
    ) -> CommandOutcome {
        match self {
            Self::Result {
                runtime, payload, ..
            } => CommandOutcome::result(
                statuses,
                AgentControlResultEnvelope::v1(
                    request_id,
                    runtime.as_ref().clone(),
                    payload.clone(),
                ),
            ),
            Self::Error {
                code, hint, detail, ..
            } => CommandOutcome::error(
                statuses,
                AgentControlErrorEnvelope::v1(
                    request_id,
                    code.clone(),
                    hint.clone(),
                    detail.clone(),
                ),
            ),
        }
    }
}

struct CommandOutcome {
    statuses: Vec<AgentControlStatusEnvelope>,
    result: Option<AgentControlResultEnvelope>,
    error: Option<AgentControlErrorEnvelope>,
}

impl CommandOutcome {
    fn result(
        statuses: Vec<AgentControlStatusEnvelope>,
        result: AgentControlResultEnvelope,
    ) -> Self {
        Self {
            statuses,
            result: Some(result),
            error: None,
        }
    }

    fn error(statuses: Vec<AgentControlStatusEnvelope>, error: AgentControlErrorEnvelope) -> Self {
        Self {
            statuses,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Clone, Debug)]
struct RuntimeProfile {
    runtime_class: Option<String>,
    region: Option<String>,
    capacity: Value,
    policy_constraints: Value,
}

fn runtime_profile(provider: ProviderKind) -> RuntimeProfile {
    RuntimeProfile {
        runtime_class: env_string_for_provider(provider, "PIKA_AGENT_RUNTIME_CLASS")
            .or_else(|| Some(provider_name(provider).to_string())),
        region: env_string_for_provider(provider, "PIKA_AGENT_RUNTIME_REGION"),
        capacity: env_json_for_provider(provider, "PIKA_AGENT_RUNTIME_CAPACITY_JSON"),
        policy_constraints: env_json_for_provider(provider, "PIKA_AGENT_RUNTIME_POLICY_JSON"),
    }
}

fn env_string_for_provider(provider: ProviderKind, key: &str) -> Option<String> {
    let provider_key = format!("{key}_{}", provider_name(provider).to_ascii_uppercase());
    std::env::var(provider_key)
        .ok()
        .or_else(|| std::env::var(key).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_json_for_provider(provider: ProviderKind, key: &str) -> Value {
    let provider_key = format!("{key}_{}", provider_name(provider).to_ascii_uppercase());
    let raw = std::env::var(provider_key)
        .ok()
        .or_else(|| std::env::var(key).ok());
    match raw {
        Some(value) if !value.trim().is_empty() => match serde_json::from_str::<Value>(&value) {
            Ok(parsed) => parsed,
            Err(_) => json!({"raw": value}),
        },
        _ => Value::Null,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DistributionPreset {
    provider: ProviderKind,
    #[serde(default)]
    runtime_class: Option<String>,
    ttl_sec: u64,
    #[serde(default)]
    region_hint: Option<String>,
    #[serde(default)]
    build_kind: Option<BuildKind>,
    #[serde(default)]
    artifact_ref: Option<String>,
    #[serde(default)]
    allowed_override_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DistributionManifest {
    distribution_ref: String,
    #[serde(default)]
    description: Option<String>,
    presets: HashMap<String, DistributionPreset>,
    #[serde(default)]
    allowed_override_keys: Vec<String>,
}

fn default_distribution_manifests() -> HashMap<String, DistributionManifest> {
    let mut presets = HashMap::new();
    presets.insert(
        "small".to_string(),
        DistributionPreset {
            provider: ProviderKind::Fly,
            runtime_class: runtime_profile(ProviderKind::Fly).runtime_class,
            ttl_sec: runtime_ttl_secs(),
            region_hint: runtime_profile(ProviderKind::Fly).region,
            build_kind: Some(BuildKind::Oci),
            artifact_ref: env_string_for_provider(
                ProviderKind::Fly,
                "PIKA_AGENT_RUNTIME_ARTIFACT_REF",
            ),
            allowed_override_keys: vec![
                "ttl_sec".to_string(),
                "region_hint".to_string(),
                "artifact_ref".to_string(),
            ],
        },
    );
    presets.insert(
        "medium".to_string(),
        DistributionPreset {
            provider: ProviderKind::Microvm,
            runtime_class: runtime_profile(ProviderKind::Microvm).runtime_class,
            ttl_sec: runtime_ttl_secs().saturating_mul(2),
            region_hint: runtime_profile(ProviderKind::Microvm).region,
            build_kind: Some(BuildKind::Nix),
            artifact_ref: env_string_for_provider(
                ProviderKind::Microvm,
                "PIKA_AGENT_RUNTIME_ARTIFACT_REF",
            ),
            allowed_override_keys: vec![
                "ttl_sec".to_string(),
                "region_hint".to_string(),
                "runtime_class".to_string(),
                "artifact_ref".to_string(),
            ],
        },
    );
    let mut manifests = HashMap::new();
    manifests.insert(
        "agent.default".to_string(),
        DistributionManifest {
            distribution_ref: "agent.default".to_string(),
            description: Some("Built-in distribution presets".to_string()),
            presets,
            allowed_override_keys: vec![],
        },
    );
    manifests
}

fn load_distribution_manifests() -> HashMap<String, DistributionManifest> {
    let raw = match std::env::var("PIKA_AGENT_CONTROL_DISTRIBUTIONS_JSON") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return default_distribution_manifests(),
    };
    let manifests: Vec<DistributionManifest> = match serde_json::from_str(&raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            warn!(
                error = %err,
                "failed to parse PIKA_AGENT_CONTROL_DISTRIBUTIONS_JSON; using default distribution manifests"
            );
            return default_distribution_manifests();
        }
    };
    let mut by_ref = HashMap::new();
    for manifest in manifests {
        by_ref.insert(manifest.distribution_ref.clone(), manifest);
    }
    if by_ref.is_empty() {
        return default_distribution_manifests();
    }
    by_ref
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BuildPhase {
    Queued,
    Validating,
    FetchingSource,
    Building,
    PublishingArtifact,
    Succeeded,
    Failed,
    Canceled,
}

impl BuildPhase {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Canceled)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BuildRecord {
    build_id: String,
    owner_pubkey_hex: String,
    build_kind: BuildKind,
    phase: BuildPhase,
    #[serde(default)]
    source_ref: Option<String>,
    #[serde(default)]
    artifact_ref: Option<String>,
    created_at: u64,
    updated_at: u64,
    deadline_at: u64,
    #[serde(default)]
    ready_at: Option<u64>,
    #[serde(default)]
    context_bytes: u64,
    #[serde(default)]
    timeout_sec: u64,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    error_detail: Option<String>,
    #[serde(default)]
    canceled_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ArtifactRecord {
    artifact_ref: String,
    build_kind: BuildKind,
    owner_pubkey_hex: String,
    created_at: u64,
    last_used_at: u64,
    expires_at: u64,
    #[serde(default)]
    source_build_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AuditEvent {
    ts: u64,
    actor_pubkey_hex: String,
    action: String,
    outcome: String,
    #[serde(default)]
    detail: Value,
}

#[derive(Clone, Debug)]
struct BuildSubmitRequest {
    build_kind: BuildKind,
    source_ref: Option<String>,
    artifact_ref: Option<String>,
    timeout_sec: u64,
    context_bytes: u64,
}

#[derive(Clone, Debug)]
enum BuildSubmitOutcome {
    ImmediateSuccess {
        artifact_ref: String,
    },
    Pending {
        next_phase: BuildPhase,
        ready_at: u64,
    },
}

#[derive(Clone, Debug)]
struct BuildPollOutcome {
    phase: BuildPhase,
    artifact_ref: Option<String>,
    error_code: Option<String>,
    error_detail: Option<String>,
}

#[async_trait]
trait BuildServiceAdapter: Send + Sync {
    async fn submit(
        &self,
        build_id: &str,
        request: &BuildSubmitRequest,
        now: u64,
    ) -> anyhow::Result<BuildSubmitOutcome>;

    async fn poll(
        &self,
        build: &BuildRecord,
        now: u64,
        allowed_source_rules: &[SourceAllowRule],
    ) -> anyhow::Result<Option<BuildPollOutcome>>;

    async fn cancel(&self, _build: &BuildRecord, _now: u64) -> anyhow::Result<()>;
}

#[derive(Clone)]
struct DefaultBuildService;

#[async_trait]
impl BuildServiceAdapter for DefaultBuildService {
    async fn submit(
        &self,
        _build_id: &str,
        request: &BuildSubmitRequest,
        now: u64,
    ) -> anyhow::Result<BuildSubmitOutcome> {
        if let Some(artifact_ref) = request.artifact_ref.as_deref() {
            ensure_immutable_artifact_ref(request.build_kind, artifact_ref)?;
            return Ok(BuildSubmitOutcome::ImmediateSuccess {
                artifact_ref: artifact_ref.to_string(),
            });
        }
        let ready_at = now.saturating_add((request.timeout_sec / 2).clamp(2, 90));
        Ok(BuildSubmitOutcome::Pending {
            next_phase: BuildPhase::FetchingSource,
            ready_at,
        })
    }

    async fn poll(
        &self,
        build: &BuildRecord,
        now: u64,
        allowed_source_rules: &[SourceAllowRule],
    ) -> anyhow::Result<Option<BuildPollOutcome>> {
        if build.phase.is_terminal() {
            return Ok(None);
        }
        if now >= build.deadline_at {
            return Ok(Some(BuildPollOutcome {
                phase: BuildPhase::Failed,
                artifact_ref: None,
                error_code: Some("build_timeout".to_string()),
                error_detail: Some("build exceeded timeout".to_string()),
            }));
        }
        let Some(source_ref) = build.source_ref.as_deref() else {
            return Ok(None);
        };
        let parsed_source = match parse_source_ref(source_ref) {
            Ok(parsed) => parsed,
            Err(err) => {
                return Ok(Some(BuildPollOutcome {
                    phase: BuildPhase::Failed,
                    artifact_ref: None,
                    error_code: Some("build_source_invalid".to_string()),
                    error_detail: Some(err.to_string()),
                }));
            }
        };
        if !allowed_source_rules
            .iter()
            .any(|rule| rule.matches(&parsed_source))
        {
            return Ok(Some(BuildPollOutcome {
                phase: BuildPhase::Failed,
                artifact_ref: None,
                error_code: Some("build_source_not_allowed".to_string()),
                error_detail: Some(format!("source_ref is outside allowlist: {source_ref}")),
            }));
        }
        let ready_at = build.ready_at.unwrap_or(build.deadline_at);
        if now < ready_at {
            let phase = if build.phase == BuildPhase::FetchingSource {
                BuildPhase::Building
            } else {
                build.phase
            };
            if phase == build.phase {
                return Ok(None);
            }
            return Ok(Some(BuildPollOutcome {
                phase,
                artifact_ref: None,
                error_code: None,
                error_detail: None,
            }));
        }
        let digest = sha256::Hash::hash(
            format!("{}:{source_ref}", build_kind_name(build.build_kind)).as_bytes(),
        )
        .to_string();
        let artifact_ref = match build.build_kind {
            BuildKind::Oci => format!("oci://builder-cache/pika-agent@sha256:{digest}"),
            BuildKind::Nix => format!("nix://closure/{digest}"),
        };
        Ok(Some(BuildPollOutcome {
            phase: BuildPhase::Succeeded,
            artifact_ref: Some(artifact_ref),
            error_code: None,
            error_detail: None,
        }))
    }

    async fn cancel(&self, _build: &BuildRecord, _now: u64) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct ProvisionedRuntime {
    provider_handle: ProviderHandle,
    bot_pubkey: Option<String>,
    metadata: Value,
    runtime_class: Option<String>,
    region: Option<String>,
    capacity: Value,
    policy_constraints: Value,
    protocol_compatibility: Vec<ProtocolKind>,
}

#[async_trait]
trait ProviderAdapter: Send + Sync {
    async fn provision(
        &self,
        runtime_id: &str,
        owner_pubkey: PublicKey,
        provision: &ProvisionCommand,
    ) -> anyhow::Result<ProvisionedRuntime>;

    async fn process_welcome(
        &self,
        runtime: &RuntimeRecord,
        process_welcome: &ProcessWelcomeCommand,
    ) -> anyhow::Result<Value>;

    async fn teardown(&self, runtime: &RuntimeRecord) -> anyhow::Result<Value>;
}

#[derive(Clone)]
struct FlyAdapter;

#[async_trait]
impl ProviderAdapter for FlyAdapter {
    async fn provision(
        &self,
        _runtime_id: &str,
        _owner_pubkey: PublicKey,
        provision: &ProvisionCommand,
    ) -> anyhow::Result<ProvisionedRuntime> {
        let profile = runtime_profile(ProviderKind::Fly);
        let fly = FlyClient::from_env()?;
        let anthropic_key =
            std::env::var("ANTHROPIC_API_KEY").context("ANTHROPIC_API_KEY must be set")?;
        let openai_key = std::env::var("OPENAI_API_KEY").ok();
        let pi_model = std::env::var("PI_MODEL")
            .ok()
            .filter(|v| !v.trim().is_empty());

        let bot_keys = if let Some(secret) = provision.bot_secret_key_hex.as_deref() {
            Keys::parse(secret).context("parse bot_secret_key_hex")?
        } else {
            Keys::generate()
        };
        let bot_pubkey = bot_keys.public_key().to_hex();
        let bot_secret_hex = bot_keys.secret_key().to_secret_hex();

        let suffix = format!("{:08x}", rand::thread_rng().r#gen::<u32>());
        let volume_name = format!("agent_{suffix}");
        let machine_name = provision
            .name
            .clone()
            .unwrap_or_else(|| format!("agent-{suffix}"));

        let volume = fly.create_volume(&volume_name).await?;
        let mut env = HashMap::new();
        env.insert("STATE_DIR".to_string(), "/app/state".to_string());
        env.insert("NOSTR_SECRET_KEY".to_string(), bot_secret_hex);
        env.insert("ANTHROPIC_API_KEY".to_string(), anthropic_key);
        if let Some(openai) = openai_key {
            env.insert("OPENAI_API_KEY".to_string(), openai);
        }
        if let Some(model) = pi_model {
            env.insert("PI_MODEL".to_string(), model);
        }
        let image_override = provision
            .artifact_ref
            .as_deref()
            .filter(|artifact_ref| parse_artifact_kind(artifact_ref) == Some(BuildKind::Oci));
        let machine = fly
            .create_machine(&machine_name, &volume.id, env, image_override)
            .await?;

        Ok(ProvisionedRuntime {
            provider_handle: ProviderHandle::Fly {
                machine_id: machine.id.clone(),
                volume_id: volume.id.clone(),
                app_name: fly.app_name().to_string(),
            },
            bot_pubkey: Some(bot_pubkey),
            metadata: json!({
                "machine_id": machine.id,
                "volume_id": volume.id,
                "app_name": fly.app_name(),
                "runtime_class": profile.runtime_class.clone(),
                "region": profile.region.clone(),
            }),
            runtime_class: profile.runtime_class,
            region: profile.region,
            capacity: profile.capacity,
            policy_constraints: profile.policy_constraints,
            protocol_compatibility: vec![ProtocolKind::Acp],
        })
    }

    async fn process_welcome(
        &self,
        _runtime: &RuntimeRecord,
        _process_welcome: &ProcessWelcomeCommand,
    ) -> anyhow::Result<Value> {
        Ok(
            json!({"processed": false, "reason": "fly runtime does not require explicit welcome hook"}),
        )
    }

    async fn teardown(&self, runtime: &RuntimeRecord) -> anyhow::Result<Value> {
        let ProviderHandle::Fly {
            machine_id,
            volume_id,
            app_name,
        } = &runtime.provider_handle
        else {
            anyhow::bail!("fly adapter received non-fly runtime handle")
        };
        let fly = FlyClient::from_env_with_app_name(app_name)?;

        let mut error_entries = Vec::new();
        if let Err(stop_err) = fly.stop_machine(machine_id).await {
            error_entries.push(json!({
                "step": "stop_machine",
                "detail": stop_err.to_string(),
                "status_code": stop_err.status_code(),
                "retryable": stop_err.is_retryable(),
            }));
        }

        let (machine_status, machine_retryable) = match fly.delete_machine(machine_id).await {
            Ok(DeleteMachineOutcome::Deleted) => ("deleted", false),
            Ok(DeleteMachineOutcome::AlreadyGone) => ("already_gone", false),
            Err(err) => {
                error_entries.push(json!({
                    "step": "delete_machine",
                    "detail": err.to_string(),
                    "status_code": err.status_code(),
                    "retryable": err.is_retryable(),
                }));
                ("failed", err.is_retryable())
            }
        };

        let (volume_status, volume_retryable) = match fly.delete_volume(volume_id).await {
            Ok(DeleteVolumeOutcome::Deleted) => ("deleted", false),
            Ok(DeleteVolumeOutcome::AlreadyGone) => ("already_gone", false),
            Ok(DeleteVolumeOutcome::Conflict) => ("conflict", true),
            Err(err) => {
                error_entries.push(json!({
                    "step": "delete_volume",
                    "detail": err.to_string(),
                    "status_code": err.status_code(),
                    "retryable": err.is_retryable(),
                }));
                ("failed", err.is_retryable())
            }
        };

        let teardown = if machine_status == "already_gone" && volume_status == "already_gone" {
            "already_gone"
        } else if machine_status == "failed" || volume_status == "failed" {
            "failed"
        } else if volume_status == "conflict" {
            "partial"
        } else {
            "deleted"
        };
        let retryable = if teardown == "partial" {
            true
        } else if teardown == "failed" {
            machine_retryable || volume_retryable
        } else {
            false
        };

        let mut payload = json!({
            "teardown": teardown,
            "machine_id": machine_id,
            "volume_id": volume_id,
            "app_name": app_name,
            "machine_status": machine_status,
            "volume_status": volume_status,
            "retryable": retryable,
        });
        if !error_entries.is_empty() {
            if let Value::Object(ref mut map) = payload {
                map.insert("error".to_string(), Value::Array(error_entries));
            }
        }
        Ok(payload)
    }
}

#[derive(Clone)]
struct MicrovmAdapter;

#[async_trait]
impl ProviderAdapter for MicrovmAdapter {
    async fn provision(
        &self,
        _runtime_id: &str,
        owner_pubkey: PublicKey,
        provision: &ProvisionCommand,
    ) -> anyhow::Result<ProvisionedRuntime> {
        let profile = runtime_profile(ProviderKind::Microvm);
        let params = provision.microvm.clone().unwrap_or_default();
        let resolved = resolve_params(&params, provision.keep);
        let relay_urls = if provision.relay_urls.is_empty() {
            default_relay_urls()
        } else {
            provision.relay_urls.clone()
        };

        let bot_keys = if let Some(secret) = provision.bot_secret_key_hex.as_deref() {
            Keys::parse(secret).context("parse bot_secret_key_hex")?
        } else {
            Keys::generate()
        };
        let bot_pubkey = bot_keys.public_key().to_hex();
        let bot_secret_hex = bot_keys.secret_key().to_secret_hex();

        let spawner = MicrovmSpawnerClient::new(resolved.spawner_url.clone());
        let create_vm = build_create_vm_request(
            &resolved,
            &owner_pubkey,
            &relay_urls,
            &bot_secret_hex,
            &bot_pubkey,
        );
        let vm = spawner
            .create_vm(&create_vm)
            .await
            .map_err(|err| spawner_create_error(&resolved.spawner_url, err))?;

        Ok(ProvisionedRuntime {
            provider_handle: ProviderHandle::Microvm {
                vm_id: vm.id.clone(),
                spawner_url: resolved.spawner_url.clone(),
                keep: resolved.keep,
            },
            bot_pubkey: Some(bot_pubkey),
            metadata: json!({
                "vm_id": vm.id,
                "vm_ip": vm.ip,
                "spawner_url": resolved.spawner_url,
                "keep": resolved.keep,
                "runtime_class": profile.runtime_class.clone(),
                "region": profile.region.clone(),
            }),
            runtime_class: profile.runtime_class,
            region: profile.region,
            capacity: profile.capacity,
            policy_constraints: profile.policy_constraints,
            protocol_compatibility: vec![ProtocolKind::Acp],
        })
    }

    async fn process_welcome(
        &self,
        _runtime: &RuntimeRecord,
        _process_welcome: &ProcessWelcomeCommand,
    ) -> anyhow::Result<Value> {
        Ok(
            json!({"processed": false, "reason": "microvm runtime receives welcome through relay flow"}),
        )
    }

    async fn teardown(&self, runtime: &RuntimeRecord) -> anyhow::Result<Value> {
        let ProviderHandle::Microvm {
            vm_id,
            spawner_url,
            keep,
        } = &runtime.provider_handle
        else {
            anyhow::bail!("microvm adapter received non-microvm runtime handle")
        };
        if *keep {
            return Ok(json!({
                "teardown": "skipped",
                "vm_id": vm_id,
                "spawner_url": spawner_url,
                "reason": "--keep policy",
            }));
        }
        let spawner = MicrovmSpawnerClient::new(spawner_url.clone());
        spawner.delete_vm(vm_id).await?;
        Ok(json!({
            "teardown": "deleted",
            "vm_id": vm_id,
            "spawner_url": spawner_url,
        }))
    }
}

fn default_relay_urls() -> Vec<String> {
    default_message_relays()
}

fn default_list_summary_descriptor() -> RuntimeDescriptor {
    RuntimeDescriptor {
        runtime_id: "runtime-list-summary".to_string(),
        provider: ProviderKind::Fly,
        lifecycle_phase: RuntimeLifecyclePhase::Ready,
        runtime_class: None,
        region: None,
        capacity: Value::Null,
        policy_constraints: Value::Null,
        protocol_compatibility: vec![],
        bot_pubkey: None,
        metadata: json!({"summary": true}),
    }
}

fn with_lease_metadata(
    existing: Value,
    created_at: u64,
    expires_at: u64,
    teardown_retry: Option<&TeardownRetryMetadata>,
) -> Value {
    let mut map = match existing {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("provider_metadata".to_string(), other);
            map
        }
    };
    map.insert("created_at".to_string(), json!(created_at));
    map.insert("expires_at".to_string(), json!(expires_at));
    if let Some(retry) = teardown_retry {
        map.insert(
            "teardown_retry".to_string(),
            serde_json::to_value(retry).unwrap_or(Value::Null),
        );
    } else {
        map.remove("teardown_retry");
    }
    Value::Object(map)
}

fn with_artifact_metadata(existing: Value, build_id: Option<&str>, artifact_ref: &str) -> Value {
    let mut map = match existing {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("provider_metadata".to_string(), other);
            map
        }
    };
    map.insert("artifact_ref".to_string(), json!(artifact_ref));
    if let Some(build_id) = build_id {
        map.insert("build_id".to_string(), json!(build_id));
    }
    Value::Object(map)
}

fn with_advanced_workload_metadata(existing: Value, advanced: Value) -> Value {
    let mut map = match existing {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("provider_metadata".to_string(), other);
            map
        }
    };
    map.insert("advanced_workload".to_string(), advanced);
    Value::Object(map)
}

fn provider_name(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Fly => "fly",
        ProviderKind::Microvm => "microvm",
    }
}

fn protocol_name(_protocol: ProtocolKind) -> &'static str {
    "acp"
}

fn build_kind_name(kind: BuildKind) -> &'static str {
    match kind {
        BuildKind::Oci => "oci",
        BuildKind::Nix => "nix",
    }
}

fn default_build_kind_for_provider(provider: ProviderKind) -> BuildKind {
    match provider {
        ProviderKind::Fly => BuildKind::Oci,
        ProviderKind::Microvm => BuildKind::Nix,
    }
}

fn normalize_submit_build_command(
    submit_build: &SubmitBuildCommand,
    build_policy: &BuildPolicy,
    advanced_workload_enabled: bool,
) -> Result<BuildSubmitRequest, (String, String, String)> {
    let has_source = submit_build
        .source_ref
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    let has_artifact = submit_build
        .artifact_ref
        .as_deref()
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false);
    if has_source == has_artifact {
        return Err((
            "build_request_invalid".to_string(),
            "set exactly one of source_ref or artifact_ref".to_string(),
            "source_ref and artifact_ref are mutually exclusive".to_string(),
        ));
    }

    let timeout_sec = submit_build
        .timeout_sec
        .unwrap_or(build_policy.default_timeout_secs)
        .clamp(60, build_policy.max_timeout_secs.max(60));
    let context_bytes = submit_build.context_bytes.unwrap_or(0);
    if context_bytes > build_policy.max_context_bytes {
        return Err((
            "build_context_too_large".to_string(),
            "reduce build context size or increase server policy limit".to_string(),
            format!(
                "context_bytes={context_bytes}, max_context_bytes={}",
                build_policy.max_context_bytes
            ),
        ));
    }

    let source_ref = submit_build
        .source_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if source_ref.is_some() && !advanced_workload_enabled {
        return Err((
            "v2_advanced_workload_disabled".to_string(),
            "set PIKA_AGENT_CONTROL_V2_ADVANCED_WORKLOAD_ENABLED=1 to allow source builds"
                .to_string(),
            "source_ref requires advanced workload mode".to_string(),
        ));
    }
    if let Some(source_ref) = source_ref.as_deref() {
        if build_policy.allowed_source_rules.is_empty() {
            return Err((
                "build_source_disabled".to_string(),
                "configure PIKA_AGENT_CONTROL_BUILD_ALLOWED_SOURCE_PREFIXES to allow source builds"
                    .to_string(),
                "no source build prefixes configured".to_string(),
            ));
        }
        let parsed_source = match parse_source_ref(source_ref) {
            Ok(parsed) => parsed,
            Err(err) => {
                return Err((
                    "build_source_invalid".to_string(),
                    "source_ref must be a normalized git+https/http url".to_string(),
                    err.to_string(),
                ));
            }
        };
        if !build_policy
            .allowed_source_rules
            .iter()
            .any(|rule| rule.matches(&parsed_source))
        {
            return Err((
                "build_source_not_allowed".to_string(),
                "source_ref must match configured allowlist prefixes".to_string(),
                source_ref.to_string(),
            ));
        }
    }

    let artifact_ref = submit_build
        .artifact_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(artifact_ref) = artifact_ref.as_deref() {
        if let Err(err) = ensure_immutable_artifact_ref(submit_build.build_kind, artifact_ref) {
            return Err((
                "artifact_ref_invalid".to_string(),
                "artifact_ref must be immutable and match build_kind".to_string(),
                err.to_string(),
            ));
        }
    }

    Ok(BuildSubmitRequest {
        build_kind: submit_build.build_kind,
        source_ref,
        artifact_ref,
        timeout_sec,
        context_bytes,
    })
}

fn ensure_immutable_artifact_ref(build_kind: BuildKind, artifact_ref: &str) -> anyhow::Result<()> {
    let normalized = artifact_ref.trim();
    if normalized.is_empty() {
        anyhow::bail!("artifact_ref must be non-empty");
    }
    match build_kind {
        BuildKind::Oci => {
            if !normalized.contains("@sha256:") {
                anyhow::bail!("oci artifact_ref must include @sha256:digest");
            }
        }
        BuildKind::Nix => {
            if !(normalized.starts_with("nix://closure/") || normalized.starts_with("nix:")) {
                anyhow::bail!("nix artifact_ref must use nix://closure/<hash> or nix:<hash>");
            }
        }
    }
    Ok(())
}

fn parse_artifact_kind(artifact_ref: &str) -> Option<BuildKind> {
    let value = artifact_ref.trim();
    if value.contains("@sha256:") || value.starts_with("oci://") {
        return Some(BuildKind::Oci);
    }
    if value.starts_with("nix://") || value.starts_with("nix:") {
        return Some(BuildKind::Nix);
    }
    None
}

fn parse_source_ref(raw: &str) -> anyhow::Result<ParsedSourceRef> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("source ref must be non-empty");
    }
    let (source_prefix, url_raw) = if let Some(rest) = trimmed.strip_prefix("git+") {
        (Some("git".to_string()), rest)
    } else {
        (None, trimmed)
    };
    let url =
        reqwest::Url::parse(url_raw).with_context(|| format!("parse source url: {trimmed}"))?;
    let scheme = url.scheme().to_ascii_lowercase();
    if !matches!(scheme.as_str(), "https" | "http") {
        anyhow::bail!("source url scheme must be http or https");
    }
    let host = url
        .host_str()
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| anyhow::anyhow!("source url host is required"))?;
    let mut path_segments = Vec::new();
    if let Some(segments) = url.path_segments() {
        for raw_segment in segments {
            let segment = raw_segment.trim();
            if segment.is_empty() || segment == "." {
                continue;
            }
            if segment == ".." {
                if path_segments.pop().is_none() {
                    anyhow::bail!("source url path attempts to traverse above root");
                }
                continue;
            }
            path_segments.push(segment.to_string());
        }
    }
    Ok(ParsedSourceRef {
        source_prefix,
        scheme,
        host,
        port: url.port_or_known_default(),
        path_segments,
    })
}

fn new_build_id() -> String {
    format!(
        "build-{:08x}{:08x}",
        rand::thread_rng().r#gen::<u32>(),
        rand::thread_rng().r#gen::<u32>()
    )
}

fn should_cache_success_result(command: &AgentControlCommand) -> bool {
    matches!(
        command,
        AgentControlCommand::Provision(_)
            | AgentControlCommand::ProcessWelcome(_)
            | AgentControlCommand::Teardown(_)
            | AgentControlCommand::SubmitBuild(_)
            | AgentControlCommand::CancelBuild(_)
    )
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn teardown_backoff_secs(attempt_count: u32) -> u64 {
    let exponent = attempt_count.saturating_sub(1).min(10);
    let base = 5u64.saturating_mul(1u64 << exponent);
    base.min(MAX_RETRY_DELAY_SECS)
}

fn runtime_due_for_reaper(runtime: &RuntimeRecord, now: u64) -> bool {
    let retry_due = runtime
        .teardown_retry
        .as_ref()
        .and_then(|metadata| metadata.next_retry_at)
        .map(|next_retry_at| next_retry_at <= now)
        .unwrap_or(false);
    let expired = runtime.expires_at > 0
        && runtime.expires_at <= now
        && runtime.descriptor.lifecycle_phase != RuntimeLifecyclePhase::Teardown;
    expired || retry_due
}

fn teardown_payload_state(payload: &Value) -> Option<&str> {
    payload.get("teardown").and_then(Value::as_str)
}

fn teardown_payload_retryable(payload: &Value) -> bool {
    payload
        .get("retryable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn teardown_payload_requires_retry(payload: &Value) -> bool {
    matches!(teardown_payload_state(payload), Some("partial" | "failed"))
        && teardown_payload_retryable(payload)
}

fn teardown_payload_is_failed(payload: &Value) -> bool {
    matches!(teardown_payload_state(payload), Some("failed"))
}

fn new_runtime_id(provider: ProviderKind) -> String {
    let mut rng = rand::thread_rng();
    format!(
        "{}-{:08x}{:08x}",
        provider_name(provider),
        rng.r#gen::<u32>(),
        rng.r#gen::<u32>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pika_agent_control_plane::AuthContext;

    fn mock_provisioned_runtime(runtime_id: &str) -> ProvisionedRuntime {
        ProvisionedRuntime {
            provider_handle: ProviderHandle::Fly {
                machine_id: "machine-1".to_string(),
                volume_id: "volume-1".to_string(),
                app_name: "app".to_string(),
            },
            bot_pubkey: Some("ab".repeat(32)),
            metadata: json!({"runtime_id": runtime_id, "mock": true}),
            runtime_class: Some("mock".to_string()),
            region: Some("local".to_string()),
            capacity: json!({"slots": 1}),
            policy_constraints: json!({"allow_keep": true}),
            protocol_compatibility: vec![ProtocolKind::Acp],
        }
    }

    #[derive(Clone)]
    struct MockAdapter {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl ProviderAdapter for MockAdapter {
        async fn provision(
            &self,
            runtime_id: &str,
            _owner_pubkey: PublicKey,
            _provision: &ProvisionCommand,
        ) -> anyhow::Result<ProvisionedRuntime> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(mock_provisioned_runtime(runtime_id))
        }

        async fn process_welcome(
            &self,
            _runtime: &RuntimeRecord,
            _process_welcome: &ProcessWelcomeCommand,
        ) -> anyhow::Result<Value> {
            Ok(json!({"ok": true}))
        }

        async fn teardown(&self, _runtime: &RuntimeRecord) -> anyhow::Result<Value> {
            Ok(json!({"ok": true}))
        }
    }

    #[derive(Clone)]
    struct CountingFailingAdapter {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl ProviderAdapter for CountingFailingAdapter {
        async fn provision(
            &self,
            _runtime_id: &str,
            _owner_pubkey: PublicKey,
            _provision: &ProvisionCommand,
        ) -> anyhow::Result<ProvisionedRuntime> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            anyhow::bail!("simulated provision failure")
        }

        async fn process_welcome(
            &self,
            _runtime: &RuntimeRecord,
            _process_welcome: &ProcessWelcomeCommand,
        ) -> anyhow::Result<Value> {
            Ok(json!({"ok": true}))
        }

        async fn teardown(&self, _runtime: &RuntimeRecord) -> anyhow::Result<Value> {
            Ok(json!({"ok": true}))
        }
    }

    #[derive(Clone)]
    struct ValidationMismatchAdapter {
        provision_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        teardown_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        runtime_class: Option<String>,
        protocol_compatibility: Vec<ProtocolKind>,
    }

    #[async_trait]
    impl ProviderAdapter for ValidationMismatchAdapter {
        async fn provision(
            &self,
            runtime_id: &str,
            _owner_pubkey: PublicKey,
            _provision: &ProvisionCommand,
        ) -> anyhow::Result<ProvisionedRuntime> {
            self.provision_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut provisioned = mock_provisioned_runtime(runtime_id);
            provisioned.runtime_class = self.runtime_class.clone();
            provisioned.protocol_compatibility = self.protocol_compatibility.clone();
            Ok(provisioned)
        }

        async fn process_welcome(
            &self,
            _runtime: &RuntimeRecord,
            _process_welcome: &ProcessWelcomeCommand,
        ) -> anyhow::Result<Value> {
            Ok(json!({"ok": true}))
        }

        async fn teardown(&self, _runtime: &RuntimeRecord) -> anyhow::Result<Value> {
            self.teardown_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(json!({"cleanup": true}))
        }
    }

    #[derive(Clone)]
    enum TeardownBehavior {
        Payload(Value),
        Error(&'static str),
    }

    #[derive(Clone)]
    struct TeardownContractAdapter {
        teardown_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        behavior: TeardownBehavior,
    }

    #[async_trait]
    impl ProviderAdapter for TeardownContractAdapter {
        async fn provision(
            &self,
            runtime_id: &str,
            _owner_pubkey: PublicKey,
            _provision: &ProvisionCommand,
        ) -> anyhow::Result<ProvisionedRuntime> {
            Ok(mock_provisioned_runtime(runtime_id))
        }

        async fn process_welcome(
            &self,
            _runtime: &RuntimeRecord,
            _process_welcome: &ProcessWelcomeCommand,
        ) -> anyhow::Result<Value> {
            Ok(json!({"ok": true}))
        }

        async fn teardown(&self, _runtime: &RuntimeRecord) -> anyhow::Result<Value> {
            self.teardown_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match &self.behavior {
                TeardownBehavior::Payload(payload) => Ok(payload.clone()),
                TeardownBehavior::Error(msg) => anyhow::bail!("{msg}"),
            }
        }
    }

    fn request_with(
        request_id: &str,
        idempotency_key: &str,
        command: AgentControlCommand,
    ) -> AgentControlCmdEnvelope {
        AgentControlCmdEnvelope::v1(
            request_id.to_string(),
            idempotency_key.to_string(),
            command,
            AuthContext::default(),
        )
    }

    fn request(command: AgentControlCommand) -> AgentControlCmdEnvelope {
        request_with("req-1", "idem-1", command)
    }

    async fn provision_runtime_for_tests(
        service: &AgentControlService,
        requester: PublicKey,
        request_id: &str,
        idempotency_key: &str,
    ) -> String {
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    request_id,
                    idempotency_key,
                    AgentControlCommand::Provision(ProvisionCommand {
                        provider: ProviderKind::Fly,
                        protocol: ProtocolKind::Acp,
                        name: None,
                        runtime_class: None,
                        relay_urls: vec![],
                        keep: false,
                        bot_secret_key_hex: None,
                        build_id: None,
                        artifact_ref: None,
                        advanced_workload_json: None,
                        microvm: None,
                    }),
                ),
            )
            .await;
        out.result
            .expect("provision should succeed")
            .runtime
            .runtime_id
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            rand::thread_rng().r#gen::<u64>()
        ))
    }

    fn test_build_policy(allowed_source_prefixes: Vec<&str>) -> BuildPolicy {
        BuildPolicy {
            max_active_builds: 8,
            max_submissions_per_hour: 30,
            max_context_bytes: 1024 * 1024,
            default_timeout_secs: 120,
            max_timeout_secs: 1200,
            artifact_ttl_secs: 300,
            max_audit_entries: 128,
            allowed_source_rules: allowed_source_prefixes
                .into_iter()
                .filter_map(|entry| SourceAllowRule::parse(entry).ok())
                .collect(),
        }
    }

    #[tokio::test]
    async fn idempotency_replay_does_not_reprovision() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let first = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        assert!(first.result.is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let replay = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        assert!(replay.result.is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(replay
            .statuses
            .iter()
            .any(|status| status.message.as_deref() == Some("idempotent replay")));
    }

    #[tokio::test]
    async fn get_runtime_returns_not_found_before_provision() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::GetRuntime(GetRuntimeCommand {
                    runtime_id: "does-not-exist".to_string(),
                })),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("expected not found error");
        assert_eq!(err.code, "runtime_not_found");
    }

    #[tokio::test]
    async fn list_runtimes_supports_filters() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        for (req_id, idem, provider) in [
            ("req-1", "idem-1", ProviderKind::Fly),
            ("req-2", "idem-2", ProviderKind::Microvm),
        ] {
            let out = service
                .handle_command(
                    &requester.to_hex(),
                    requester,
                    request_with(
                        req_id,
                        idem,
                        AgentControlCommand::Provision(ProvisionCommand {
                            provider,
                            protocol: ProtocolKind::Acp,
                            name: None,
                            runtime_class: None,
                            relay_urls: vec![],
                            keep: false,
                            bot_secret_key_hex: None,
                            build_id: None,
                            artifact_ref: None,
                            advanced_workload_json: None,
                            microvm: None,
                        }),
                    ),
                )
                .await;
            assert!(out.result.is_some());
        }

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-list",
                    "idem-list",
                    AgentControlCommand::ListRuntimes(ListRuntimesCommand {
                        provider: Some(ProviderKind::Microvm),
                        protocol: Some(ProtocolKind::Acp),
                        lifecycle_phase: Some(RuntimeLifecyclePhase::Ready),
                        runtime_class: Some("mock".to_string()),
                        limit: Some(10),
                    }),
                ),
            )
            .await;
        let result = out.result.expect("list result");
        let runtimes = result
            .payload
            .get("runtimes")
            .and_then(Value::as_array)
            .cloned()
            .expect("runtimes array");
        assert_eq!(runtimes.len(), 1);
        let provider = runtimes[0]
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert_eq!(provider, "microvm");
    }

    #[tokio::test]
    async fn runtime_class_mismatch_fails_before_provision() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: Some("not-fly".to_string()),
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("runtime class mismatch");
        assert_eq!(err.code, "runtime_class_unavailable");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn post_provision_protocol_validation_triggers_cleanup() {
        let provision_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(ValidationMismatchAdapter {
            provision_calls: provision_calls.clone(),
            teardown_calls: teardown_calls.clone(),
            runtime_class: runtime_profile(ProviderKind::Fly).runtime_class,
            protocol_compatibility: vec![],
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("protocol mismatch error");
        assert_eq!(err.code, "unsupported_protocol");
        assert_eq!(provision_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let state = service.state.read().await;
        assert!(state.runtimes.is_empty());
    }

    #[tokio::test]
    async fn post_provision_runtime_class_validation_triggers_cleanup() {
        let provision_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requested_class = runtime_profile(ProviderKind::Fly)
            .runtime_class
            .unwrap_or_else(|| "fly".to_string());
        let adapter = std::sync::Arc::new(ValidationMismatchAdapter {
            provision_calls: provision_calls.clone(),
            teardown_calls: teardown_calls.clone(),
            runtime_class: Some(format!("{requested_class}-actual")),
            protocol_compatibility: vec![ProtocolKind::Acp],
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: Some(requested_class),
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("runtime class mismatch error");
        assert_eq!(err.code, "runtime_class_unavailable");
        assert_eq!(provision_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let state = service.state.read().await;
        assert!(state.runtimes.is_empty());
    }

    #[tokio::test]
    async fn get_runtime_is_scoped_to_owner() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let owner = Keys::generate().public_key();
        let other = Keys::generate().public_key();

        let provision = service
            .handle_command(
                &owner.to_hex(),
                owner,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        let runtime_id = provision
            .result
            .expect("provision result")
            .runtime
            .runtime_id;

        let out = service
            .handle_command(
                &other.to_hex(),
                other,
                request_with(
                    "req-2",
                    "idem-2",
                    AgentControlCommand::GetRuntime(GetRuntimeCommand { runtime_id }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("owner scoping error");
        assert_eq!(err.code, "runtime_not_found");
    }

    #[tokio::test]
    async fn list_runtimes_is_scoped_to_owner() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let owner = Keys::generate().public_key();
        let other = Keys::generate().public_key();

        let out = service
            .handle_command(
                &owner.to_hex(),
                owner,
                request_with(
                    "req-1",
                    "idem-1",
                    AgentControlCommand::Provision(ProvisionCommand {
                        provider: ProviderKind::Fly,
                        protocol: ProtocolKind::Acp,
                        name: None,
                        runtime_class: None,
                        relay_urls: vec![],
                        keep: false,
                        bot_secret_key_hex: None,
                        build_id: None,
                        artifact_ref: None,
                        advanced_workload_json: None,
                        microvm: None,
                    }),
                ),
            )
            .await;
        assert!(out.result.is_some());

        let out = service
            .handle_command(
                &other.to_hex(),
                other,
                request_with(
                    "req-list",
                    "idem-list",
                    AgentControlCommand::ListRuntimes(ListRuntimesCommand::default()),
                ),
            )
            .await;
        let result = out.result.expect("list result");
        let count = result
            .payload
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn idempotent_error_outcomes_are_not_cached() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(CountingFailingAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let cmd = request(AgentControlCommand::Provision(ProvisionCommand {
            provider: ProviderKind::Fly,
            protocol: ProtocolKind::Acp,
            name: None,
            runtime_class: None,
            relay_urls: vec![],
            keep: false,
            bot_secret_key_hex: None,
            build_id: None,
            artifact_ref: None,
            advanced_workload_json: None,
            microvm: None,
        }));

        let first = service
            .handle_command(&requester.to_hex(), requester, cmd.clone())
            .await;
        assert!(first.error.is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let replay = service
            .handle_command(&requester.to_hex(), requester, cmd)
            .await;
        assert!(replay.error.is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(!replay
            .statuses
            .iter()
            .any(|status| status.message.as_deref() == Some("idempotent replay")));
    }

    #[tokio::test]
    async fn provision_is_denied_when_policy_disallows_requester() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let allowed = Keys::generate().public_key();
        let denied = Keys::generate().public_key();
        let service = AgentControlService::with_adapters_and_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::Allowlist(HashSet::from([allowed.to_hex()])),
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
        );

        let out = service
            .handle_command(
                &denied.to_hex(),
                denied,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("provision should be unauthorized");
        assert_eq!(err.code, "provision_unauthorized");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        let state = service.state.read().await;
        assert!(state.idempotency.is_empty());
    }

    #[tokio::test]
    async fn idempotency_cache_is_bounded() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_and_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            2,
        );
        let requester = Keys::generate().public_key();

        for i in 0..3 {
            let req_id = format!("req-{i}");
            let idem = format!("idem-{i}");
            let out = service
                .handle_command(
                    &requester.to_hex(),
                    requester,
                    request_with(
                        &req_id,
                        &idem,
                        AgentControlCommand::Provision(ProvisionCommand {
                            provider: ProviderKind::Fly,
                            protocol: ProtocolKind::Acp,
                            name: None,
                            runtime_class: None,
                            relay_urls: vec![],
                            keep: false,
                            bot_secret_key_hex: None,
                            build_id: None,
                            artifact_ref: None,
                            advanced_workload_json: None,
                            microvm: None,
                        }),
                    ),
                )
                .await;
            assert!(out.result.is_some());
        }

        let state = service.state.read().await;
        assert_eq!(state.idempotency.len(), 2);
        assert_eq!(state.idempotency_order.len(), 2);
        assert!(!state
            .idempotency
            .contains_key(&(requester.to_hex(), "idem-0".to_string())));
    }

    #[tokio::test]
    async fn teardown_persist_failure_keeps_runtime_in_teardown_phase() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let state_dir = unique_temp_path("pika-agent-control-state-dir");
        std::fs::create_dir_all(&state_dir).expect("create failing persistence directory");
        let persistence = std::sync::Arc::new(ControlStatePersistence::new(state_dir.clone()));
        let service = AgentControlService::with_adapters_policy_and_persistence(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            persistence,
        );
        let owner = Keys::generate().public_key();
        let runtime_id = "runtime-teardown-persist".to_string();
        {
            let mut state = service.state.write().await;
            state.runtimes.insert(
                runtime_id.clone(),
                RuntimeRecord {
                    owner_pubkey_hex: owner.to_hex(),
                    descriptor: RuntimeDescriptor {
                        runtime_id: runtime_id.clone(),
                        provider: ProviderKind::Fly,
                        lifecycle_phase: RuntimeLifecyclePhase::Ready,
                        runtime_class: Some("fly".to_string()),
                        region: Some("local".to_string()),
                        capacity: json!({"slots": 1}),
                        policy_constraints: Value::Null,
                        protocol_compatibility: vec![ProtocolKind::Acp],
                        bot_pubkey: Some("ab".repeat(32)),
                        metadata: Value::Null,
                    },
                    provider_handle: ProviderHandle::Fly {
                        machine_id: "machine-1".to_string(),
                        volume_id: "volume-1".to_string(),
                        app_name: "app".to_string(),
                    },
                    created_at: unix_now_secs(),
                    expires_at: unix_now_secs().saturating_add(runtime_ttl_secs()),
                    teardown_retry: None,
                },
            );
        }

        let out = service
            .handle_command(
                &owner.to_hex(),
                owner,
                request(AgentControlCommand::Teardown(TeardownCommand {
                    runtime_id: runtime_id.clone(),
                })),
            )
            .await;
        let result = out.result.expect("teardown should still return result");
        assert_eq!(
            result.runtime.lifecycle_phase,
            RuntimeLifecyclePhase::Teardown
        );
        assert_eq!(
            result.payload.get("state_persist").and_then(Value::as_str),
            Some("failed")
        );
        let state = service.state.read().await;
        assert_eq!(
            state
                .runtimes
                .get(&runtime_id)
                .expect("runtime remains in state")
                .descriptor
                .lifecycle_phase,
            RuntimeLifecyclePhase::Teardown
        );

        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn loads_legacy_runtime_state_without_owner_field() {
        let path = unique_temp_path("pika-agent-control-state.json");
        let runtime_id = "runtime-legacy-ownerless".to_string();
        let mut state = ControlState::default();
        state.runtimes.insert(
            runtime_id.clone(),
            RuntimeRecord {
                owner_pubkey_hex: "owner".to_string(),
                descriptor: RuntimeDescriptor {
                    runtime_id: runtime_id.clone(),
                    provider: ProviderKind::Fly,
                    lifecycle_phase: RuntimeLifecyclePhase::Ready,
                    runtime_class: Some("fly".to_string()),
                    region: Some("local".to_string()),
                    capacity: json!({"slots": 1}),
                    policy_constraints: Value::Null,
                    protocol_compatibility: vec![ProtocolKind::Acp],
                    bot_pubkey: Some("ab".repeat(32)),
                    metadata: Value::Null,
                },
                provider_handle: ProviderHandle::Fly {
                    machine_id: "machine-1".to_string(),
                    volume_id: "volume-1".to_string(),
                    app_name: "app".to_string(),
                },
                created_at: 0,
                expires_at: 0,
                teardown_retry: None,
            },
        );
        let mut serialized =
            serde_json::to_value(PersistedControlState::from(&state)).expect("serialize state");
        let runtimes = serialized
            .get_mut("runtimes")
            .and_then(Value::as_object_mut)
            .expect("runtimes map");
        let runtime = runtimes
            .get_mut(&runtime_id)
            .and_then(Value::as_object_mut)
            .expect("runtime entry");
        runtime.remove("owner_pubkey_hex");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serialized).expect("serialize legacy json"),
        )
        .expect("write legacy state");

        let persistence = ControlStatePersistence::new(path.clone());
        let loaded = persistence.load().expect("load legacy state");
        assert_eq!(loaded.runtimes.len(), 1);
        assert_eq!(
            loaded
                .runtimes
                .get(&runtime_id)
                .expect("legacy runtime")
                .owner_pubkey_hex,
            ""
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn loads_legacy_runtime_state_with_pi_protocol_value() {
        let path = unique_temp_path("pika-agent-control-state.json");
        let runtime_id = "runtime-legacy-pi-protocol".to_string();
        let mut state = ControlState::default();
        state.runtimes.insert(
            runtime_id.clone(),
            RuntimeRecord {
                owner_pubkey_hex: "owner".to_string(),
                descriptor: RuntimeDescriptor {
                    runtime_id: runtime_id.clone(),
                    provider: ProviderKind::Fly,
                    lifecycle_phase: RuntimeLifecyclePhase::Ready,
                    runtime_class: Some("fly".to_string()),
                    region: Some("local".to_string()),
                    capacity: json!({"slots": 1}),
                    policy_constraints: Value::Null,
                    protocol_compatibility: vec![ProtocolKind::Acp],
                    bot_pubkey: Some("ab".repeat(32)),
                    metadata: Value::Null,
                },
                provider_handle: ProviderHandle::Fly {
                    machine_id: "machine-1".to_string(),
                    volume_id: "volume-1".to_string(),
                    app_name: "app".to_string(),
                },
                created_at: 0,
                expires_at: 0,
                teardown_retry: None,
            },
        );
        let mut serialized =
            serde_json::to_value(PersistedControlState::from(&state)).expect("serialize state");
        let protocols = serialized["runtimes"][&runtime_id]["descriptor"]["protocol_compatibility"]
            .as_array_mut()
            .expect("protocol_compatibility array");
        protocols[0] = json!("pi");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serialized).expect("serialize legacy json"),
        )
        .expect("write legacy state");

        let persistence = ControlStatePersistence::new(path.clone());
        let loaded = persistence.load().expect("load legacy state");
        let runtime = loaded.runtimes.get(&runtime_id).expect("legacy runtime");
        assert_eq!(
            runtime.descriptor.protocol_compatibility,
            vec![ProtocolKind::Acp]
        );

        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn legacy_ownerless_runtime_is_accessible_to_allowlisted_requesters() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let allowed = Keys::generate().public_key();
        let denied = Keys::generate().public_key();
        let service = AgentControlService::with_adapters_and_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::Allowlist(HashSet::from([allowed.to_hex()])),
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
        );
        let runtime_id = "runtime-legacy-access".to_string();
        {
            let mut state = service.state.write().await;
            state.runtimes.insert(
                runtime_id.clone(),
                RuntimeRecord {
                    owner_pubkey_hex: String::new(),
                    descriptor: RuntimeDescriptor {
                        runtime_id: runtime_id.clone(),
                        provider: ProviderKind::Fly,
                        lifecycle_phase: RuntimeLifecyclePhase::Ready,
                        runtime_class: Some("fly".to_string()),
                        region: Some("local".to_string()),
                        capacity: json!({"slots": 1}),
                        policy_constraints: Value::Null,
                        protocol_compatibility: vec![ProtocolKind::Acp],
                        bot_pubkey: Some("ab".repeat(32)),
                        metadata: Value::Null,
                    },
                    provider_handle: ProviderHandle::Fly {
                        machine_id: "machine-1".to_string(),
                        volume_id: "volume-1".to_string(),
                        app_name: "app".to_string(),
                    },
                    created_at: unix_now_secs(),
                    expires_at: unix_now_secs().saturating_add(runtime_ttl_secs()),
                    teardown_retry: None,
                },
            );
        }

        let allowed_get = service
            .handle_command(
                &allowed.to_hex(),
                allowed,
                request_with(
                    "req-allowed",
                    "idem-allowed",
                    AgentControlCommand::GetRuntime(GetRuntimeCommand {
                        runtime_id: runtime_id.clone(),
                    }),
                ),
            )
            .await;
        assert!(allowed_get.result.is_some());

        let denied_get = service
            .handle_command(
                &denied.to_hex(),
                denied,
                request_with(
                    "req-denied",
                    "idem-denied",
                    AgentControlCommand::GetRuntime(GetRuntimeCommand { runtime_id }),
                ),
            )
            .await;
        assert!(denied_get.result.is_none());
        assert_eq!(
            denied_get.error.expect("denied should fail").code,
            "runtime_not_found"
        );
    }

    #[test]
    fn provider_name_returns_expected_strings() {
        assert_eq!(provider_name(ProviderKind::Fly), "fly");
        assert_eq!(provider_name(ProviderKind::Microvm), "microvm");
    }

    #[test]
    fn runtime_profile_defaults_to_provider_name_as_runtime_class() {
        // Ensure no env vars are set for this test
        std::env::remove_var("PIKA_AGENT_RUNTIME_CLASS");
        std::env::remove_var("PIKA_AGENT_RUNTIME_CLASS_FLY");
        std::env::remove_var("PIKA_AGENT_RUNTIME_CLASS_MICROVM");

        let fly_profile = runtime_profile(ProviderKind::Fly);
        assert_eq!(fly_profile.runtime_class, Some("fly".to_string()));
        assert_eq!(fly_profile.region, None);
        assert_eq!(fly_profile.capacity, Value::Null);
        assert_eq!(fly_profile.policy_constraints, Value::Null);

        let microvm_profile = runtime_profile(ProviderKind::Microvm);
        assert_eq!(microvm_profile.runtime_class, Some("microvm".to_string()));
    }

    #[test]
    fn env_string_for_provider_prefers_provider_specific_key() {
        let unique = format!("TEST_ENV_STR_{}", rand::thread_rng().r#gen::<u32>());
        let fly_key = format!("{unique}_FLY");

        std::env::set_var(&unique, "generic");
        std::env::set_var(&fly_key, "fly-specific");

        let result = env_string_for_provider(ProviderKind::Fly, &unique);
        assert_eq!(result, Some("fly-specific".to_string()));

        std::env::remove_var(&unique);
        std::env::remove_var(&fly_key);
    }

    #[test]
    fn env_string_for_provider_falls_back_to_generic_key() {
        let unique = format!("TEST_ENV_FALLBACK_{}", rand::thread_rng().r#gen::<u32>());
        let microvm_key = format!("{unique}_MICROVM");

        std::env::set_var(&unique, "generic-value");
        std::env::remove_var(&microvm_key);

        let result = env_string_for_provider(ProviderKind::Microvm, &unique);
        assert_eq!(result, Some("generic-value".to_string()));

        std::env::remove_var(&unique);
    }

    #[test]
    fn env_string_for_provider_returns_none_when_unset() {
        let unique = format!("TEST_ENV_NONE_{}", rand::thread_rng().r#gen::<u32>());
        let fly_key = format!("{unique}_FLY");
        std::env::remove_var(&unique);
        std::env::remove_var(&fly_key);

        let result = env_string_for_provider(ProviderKind::Fly, &unique);
        assert_eq!(result, None);
    }

    #[test]
    fn env_json_for_provider_parses_valid_json() {
        let unique = format!("TEST_ENV_JSON_{}", rand::thread_rng().r#gen::<u32>());
        std::env::set_var(&unique, r#"{"slots": 4}"#);

        let result = env_json_for_provider(ProviderKind::Fly, &unique);
        assert_eq!(result, json!({"slots": 4}));

        std::env::remove_var(&unique);
    }

    #[test]
    fn env_json_for_provider_wraps_invalid_json() {
        let unique = format!("TEST_ENV_BADJSON_{}", rand::thread_rng().r#gen::<u32>());
        std::env::set_var(&unique, "not-json");

        let result = env_json_for_provider(ProviderKind::Fly, &unique);
        assert_eq!(result, json!({"raw": "not-json"}));

        std::env::remove_var(&unique);
    }

    #[test]
    fn env_json_for_provider_returns_null_when_unset() {
        let unique = format!("TEST_ENV_NOJSON_{}", rand::thread_rng().r#gen::<u32>());
        std::env::remove_var(&unique);

        let result = env_json_for_provider(ProviderKind::Fly, &unique);
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn different_idempotency_key_triggers_new_provision() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let first = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-a",
                    "idem-a",
                    AgentControlCommand::Provision(ProvisionCommand {
                        provider: ProviderKind::Fly,
                        protocol: ProtocolKind::Acp,
                        name: None,
                        runtime_class: None,
                        relay_urls: vec![],
                        keep: false,
                        bot_secret_key_hex: None,
                        build_id: None,
                        artifact_ref: None,
                        advanced_workload_json: None,
                        microvm: None,
                    }),
                ),
            )
            .await;
        assert!(first.result.is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let second = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-b",
                    "idem-b",
                    AgentControlCommand::Provision(ProvisionCommand {
                        provider: ProviderKind::Fly,
                        protocol: ProtocolKind::Acp,
                        name: None,
                        runtime_class: None,
                        relay_urls: vec![],
                        keep: false,
                        bot_secret_key_hex: None,
                        build_id: None,
                        artifact_ref: None,
                        advanced_workload_json: None,
                        microvm: None,
                    }),
                ),
            )
            .await;
        assert!(second.result.is_some());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn provision_failure_normalizes_error_code() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(CountingFailingAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("expected provision_failed error");
        assert_eq!(err.code, "provision_failed");
        assert!(err.hint.is_some());
        assert!(
            err.detail
                .as_deref()
                .unwrap()
                .contains("simulated provision failure"),
            "raw error should be wrapped in detail field"
        );
        assert!(out
            .statuses
            .iter()
            .any(|s| s.phase == RuntimeLifecyclePhase::Failed));
    }

    #[tokio::test]
    async fn all_error_codes_are_consistent_strings() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let not_found = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-nf",
                    "idem-nf",
                    AgentControlCommand::GetRuntime(GetRuntimeCommand {
                        runtime_id: "nonexistent".to_string(),
                    }),
                ),
            )
            .await;
        let teardown_nf = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-td",
                    "idem-td",
                    AgentControlCommand::Teardown(TeardownCommand {
                        runtime_id: "nonexistent".to_string(),
                    }),
                ),
            )
            .await;

        let nf_err = not_found.error.expect("get should fail");
        let td_err = teardown_nf.error.expect("teardown should fail");
        assert_eq!(nf_err.code, "runtime_not_found");
        assert_eq!(td_err.code, "runtime_not_found");
    }

    #[tokio::test]
    async fn descriptor_fields_flow_through_provision_result() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;

        let result = out.result.expect("provision should succeed");
        let rt = &result.runtime;
        assert_eq!(rt.provider, ProviderKind::Fly);
        assert_eq!(rt.lifecycle_phase, RuntimeLifecyclePhase::Ready);
        assert_eq!(rt.runtime_class, Some("mock".to_string()));
        assert_eq!(rt.region, Some("local".to_string()));
        assert_eq!(rt.capacity, json!({"slots": 1}));
        assert_eq!(rt.policy_constraints, json!({"allow_keep": true}));
        assert_eq!(rt.protocol_compatibility, vec![ProtocolKind::Acp]);
        assert_eq!(rt.bot_pubkey, Some("ab".repeat(32)));
        assert!(!rt.metadata.is_null());
    }

    #[tokio::test]
    async fn get_runtime_returns_full_descriptor_after_provision() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let provision_out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        let provisioned_rt = provision_out
            .result
            .expect("provision should succeed")
            .runtime;
        let runtime_id = provisioned_rt.runtime_id.clone();

        let get_out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-get",
                    "idem-get",
                    AgentControlCommand::GetRuntime(GetRuntimeCommand {
                        runtime_id: runtime_id.clone(),
                    }),
                ),
            )
            .await;
        let get_rt = get_out.result.expect("get should succeed").runtime;
        assert_eq!(get_rt.runtime_id, runtime_id);
        assert_eq!(get_rt.provider, provisioned_rt.provider);
        assert_eq!(get_rt.runtime_class, provisioned_rt.runtime_class);
        assert_eq!(get_rt.region, provisioned_rt.region);
        assert_eq!(get_rt.bot_pubkey, provisioned_rt.bot_pubkey);
        assert_eq!(
            get_rt.protocol_compatibility,
            provisioned_rt.protocol_compatibility
        );
    }

    #[tokio::test]
    async fn teardown_transitions_lifecycle_phase() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();

        let provision_out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request(AgentControlCommand::Provision(ProvisionCommand {
                    provider: ProviderKind::Fly,
                    protocol: ProtocolKind::Acp,
                    name: None,
                    runtime_class: None,
                    relay_urls: vec![],
                    keep: false,
                    bot_secret_key_hex: None,
                    build_id: None,
                    artifact_ref: None,
                    advanced_workload_json: None,
                    microvm: None,
                })),
            )
            .await;
        let runtime_id = provision_out.result.expect("provision").runtime.runtime_id;

        let teardown_out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-td",
                    "idem-td",
                    AgentControlCommand::Teardown(TeardownCommand {
                        runtime_id: runtime_id.clone(),
                    }),
                ),
            )
            .await;
        let td_rt = teardown_out
            .result
            .expect("teardown should succeed")
            .runtime;
        assert_eq!(td_rt.runtime_id, runtime_id);
        assert_eq!(td_rt.lifecycle_phase, RuntimeLifecyclePhase::Teardown);

        let get_after = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-get2",
                    "idem-get2",
                    AgentControlCommand::GetRuntime(GetRuntimeCommand {
                        runtime_id: runtime_id.clone(),
                    }),
                ),
            )
            .await;
        let get_rt = get_after.result.expect("get after teardown").runtime;
        assert_eq!(get_rt.lifecycle_phase, RuntimeLifecyclePhase::Teardown);
    }

    #[tokio::test]
    async fn teardown_contract_deleted_payload_returns_success() {
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: teardown_calls.clone(),
            behavior: TeardownBehavior::Payload(json!({
                "teardown": "deleted",
                "machine_status": "deleted",
                "volume_status": "deleted",
                "retryable": false,
            })),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let runtime_id =
            provision_runtime_for_tests(&service, requester, "req-provision", "idem-provision")
                .await;

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-teardown",
                    "idem-teardown",
                    AgentControlCommand::Teardown(TeardownCommand { runtime_id }),
                ),
            )
            .await;
        let result = out.result.expect("teardown should return success");
        assert_eq!(
            result.payload.get("teardown").and_then(Value::as_str),
            Some("deleted")
        );
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn teardown_contract_already_gone_payload_returns_success() {
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: teardown_calls.clone(),
            behavior: TeardownBehavior::Payload(json!({
                "teardown": "already_gone",
                "machine_status": "already_gone",
                "volume_status": "already_gone",
                "retryable": false,
            })),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let runtime_id =
            provision_runtime_for_tests(&service, requester, "req-provision", "idem-provision")
                .await;

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-teardown",
                    "idem-teardown",
                    AgentControlCommand::Teardown(TeardownCommand { runtime_id }),
                ),
            )
            .await;
        let result = out.result.expect("teardown should return success");
        assert_eq!(
            result.payload.get("teardown").and_then(Value::as_str),
            Some("already_gone")
        );
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn teardown_contract_partial_conflict_schedules_retry() {
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: teardown_calls.clone(),
            behavior: TeardownBehavior::Payload(json!({
                "teardown": "partial",
                "machine_status": "deleted",
                "volume_status": "conflict",
                "retryable": true,
            })),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let runtime_id =
            provision_runtime_for_tests(&service, requester, "req-provision", "idem-provision")
                .await;

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-teardown",
                    "idem-teardown",
                    AgentControlCommand::Teardown(TeardownCommand {
                        runtime_id: runtime_id.clone(),
                    }),
                ),
            )
            .await;
        let result = out.result.expect("partial teardown should return result");
        assert_eq!(
            result.payload.get("teardown").and_then(Value::as_str),
            Some("partial")
        );
        let state = service.state.read().await;
        let retry = state
            .runtimes
            .get(&runtime_id)
            .and_then(|runtime| runtime.teardown_retry.as_ref())
            .expect("retry metadata should be stored");
        assert_eq!(retry.attempt_count, 1);
        assert!(retry.next_retry_at.is_some());
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn teardown_contract_failed_payload_returns_error() {
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: teardown_calls.clone(),
            behavior: TeardownBehavior::Payload(json!({
                "teardown": "failed",
                "machine_status": "failed",
                "volume_status": "failed",
                "retryable": true,
            })),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let runtime_id =
            provision_runtime_for_tests(&service, requester, "req-provision", "idem-provision")
                .await;

        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-teardown",
                    "idem-teardown",
                    AgentControlCommand::Teardown(TeardownCommand {
                        runtime_id: runtime_id.clone(),
                    }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("failed teardown should return error");
        assert_eq!(err.code, "teardown_failed");
        let state = service.state.read().await;
        let retry = state
            .runtimes
            .get(&runtime_id)
            .and_then(|runtime| runtime.teardown_retry.as_ref())
            .expect("retry metadata should be stored");
        assert_eq!(retry.attempt_count, 1);
        assert!(retry.next_retry_at.is_some());
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reaper_expires_runtime_and_runs_teardown() {
        let now = 1_700_000_000u64;
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: teardown_calls.clone(),
            behavior: TeardownBehavior::Payload(json!({
                "teardown": "deleted",
                "machine_status": "deleted",
                "volume_status": "deleted",
                "retryable": false,
            })),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let runtime_id =
            provision_runtime_for_tests(&service, requester, "req-provision", "idem-provision")
                .await;

        {
            let mut state = service.state.write().await;
            let runtime = state.runtimes.get_mut(&runtime_id).expect("runtime exists");
            runtime.expires_at = now.saturating_sub(1);
            runtime.descriptor.lifecycle_phase = RuntimeLifecyclePhase::Ready;
        }
        let processed = service
            .reap_expired_runtimes_for_test(now)
            .await
            .expect("reaper succeeds");
        assert_eq!(processed, 1);
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let state = service.state.read().await;
        let runtime = state
            .runtimes
            .get(&runtime_id)
            .expect("runtime still present");
        assert_eq!(
            runtime.descriptor.lifecycle_phase,
            RuntimeLifecyclePhase::Teardown
        );
        assert!(runtime.teardown_retry.is_none());
    }

    #[tokio::test]
    async fn reaper_schedules_retry_when_teardown_errors() {
        let now = 1_700_000_010u64;
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: teardown_calls.clone(),
            behavior: TeardownBehavior::Error("simulated provider outage"),
        });
        let service = AgentControlService::with_adapters(adapter.clone(), adapter);
        let requester = Keys::generate().public_key();
        let runtime_id =
            provision_runtime_for_tests(&service, requester, "req-provision", "idem-provision")
                .await;

        {
            let mut state = service.state.write().await;
            let runtime = state.runtimes.get_mut(&runtime_id).expect("runtime exists");
            runtime.expires_at = now.saturating_sub(1);
        }
        service
            .reap_expired_runtimes_for_test(now)
            .await
            .expect("reaper succeeds");
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let state = service.state.read().await;
        let retry = state
            .runtimes
            .get(&runtime_id)
            .and_then(|runtime| runtime.teardown_retry.as_ref())
            .expect("retry metadata should be set");
        assert_eq!(retry.attempt_count, 1);
        assert_eq!(retry.next_retry_at, Some(now.saturating_add(5)));
    }

    #[tokio::test]
    async fn reaper_transition_persist_failure_marks_runtime_retryable() {
        let now = 1_700_000_020u64;
        let teardown_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: teardown_calls.clone(),
            behavior: TeardownBehavior::Payload(json!({
                "teardown": "deleted",
                "machine_status": "deleted",
                "volume_status": "deleted",
                "retryable": false,
            })),
        });
        let state_dir = unique_temp_path("pika-agent-control-reaper-persist-dir");
        std::fs::create_dir_all(&state_dir).expect("create failing persistence directory");
        let persistence = std::sync::Arc::new(ControlStatePersistence::new(state_dir.clone()));
        let service = AgentControlService::with_adapters_policy_and_persistence(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            persistence,
        );
        let owner = Keys::generate().public_key();
        let runtime_id = "runtime-reaper-persist-failure".to_string();

        {
            let mut state = service.state.write().await;
            state.runtimes.insert(
                runtime_id.clone(),
                RuntimeRecord {
                    owner_pubkey_hex: owner.to_hex(),
                    descriptor: RuntimeDescriptor {
                        runtime_id: runtime_id.clone(),
                        provider: ProviderKind::Fly,
                        lifecycle_phase: RuntimeLifecyclePhase::Ready,
                        runtime_class: Some("fly".to_string()),
                        region: Some("local".to_string()),
                        capacity: json!({"slots": 1}),
                        policy_constraints: Value::Null,
                        protocol_compatibility: vec![ProtocolKind::Acp],
                        bot_pubkey: Some("ab".repeat(32)),
                        metadata: Value::Null,
                    },
                    provider_handle: ProviderHandle::Fly {
                        machine_id: "machine-1".to_string(),
                        volume_id: "volume-1".to_string(),
                        app_name: "app".to_string(),
                    },
                    created_at: now.saturating_sub(120),
                    expires_at: now.saturating_sub(1),
                    teardown_retry: None,
                },
            );
        }

        service
            .reap_expired_runtimes_for_test(now)
            .await
            .expect("reaper run should complete");
        assert_eq!(teardown_calls.load(std::sync::atomic::Ordering::SeqCst), 0);

        let state = service.state.read().await;
        let runtime = state.runtimes.get(&runtime_id).expect("runtime exists");
        assert_eq!(
            runtime.descriptor.lifecycle_phase,
            RuntimeLifecyclePhase::Teardown
        );
        let retry = runtime
            .teardown_retry
            .as_ref()
            .expect("persist failure should schedule retry metadata");
        assert_eq!(retry.next_retry_at, Some(now));

        let _ = std::fs::remove_dir_all(&state_dir);
    }

    #[tokio::test]
    async fn reaper_restart_resumes_pending_retry() {
        let now = 1_700_000_050u64;
        let state_path = unique_temp_path("pika-agent-control-reaper-restart.json");
        let persistence = std::sync::Arc::new(ControlStatePersistence::new(state_path.clone()));

        let failing_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let failing_adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: failing_calls.clone(),
            behavior: TeardownBehavior::Error("transient teardown error"),
        });
        let service_a = AgentControlService::with_adapters_policy_and_persistence(
            failing_adapter.clone(),
            failing_adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            persistence.clone(),
        );
        let requester = Keys::generate().public_key();
        let runtime_id =
            provision_runtime_for_tests(&service_a, requester, "req-provision", "idem-provision")
                .await;
        {
            let mut state = service_a.state.write().await;
            let runtime = state.runtimes.get_mut(&runtime_id).expect("runtime exists");
            runtime.expires_at = now.saturating_sub(1);
        }
        service_a
            .reap_expired_runtimes_for_test(now)
            .await
            .expect("first reaper run succeeds");
        assert_eq!(failing_calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let succeeding_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let succeeding_adapter = std::sync::Arc::new(TeardownContractAdapter {
            teardown_calls: succeeding_calls.clone(),
            behavior: TeardownBehavior::Payload(json!({
                "teardown": "deleted",
                "machine_status": "deleted",
                "volume_status": "deleted",
                "retryable": false,
            })),
        });
        let service_b = AgentControlService::with_adapters_policy_and_loaded_persistence(
            succeeding_adapter.clone(),
            succeeding_adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            persistence.clone(),
        )
        .expect("load service from persisted state");
        let retry_at = {
            let state = service_b.state.read().await;
            state
                .runtimes
                .get(&runtime_id)
                .and_then(|runtime| runtime.teardown_retry.as_ref())
                .and_then(|retry| retry.next_retry_at)
                .expect("persisted retry metadata")
        };
        service_b
            .reap_expired_runtimes_for_test(retry_at)
            .await
            .expect("second reaper run succeeds");
        assert_eq!(
            succeeding_calls.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        let state = service_b.state.read().await;
        let runtime = state.runtimes.get(&runtime_id).expect("runtime exists");
        assert!(runtime.teardown_retry.is_none());

        let _ = std::fs::remove_file(state_path);
    }

    #[tokio::test]
    async fn v2_submit_build_is_available_without_phase_gates() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );
        let requester = Keys::generate().public_key();
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-submit",
                    "idem-build-submit",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: None,
                        artifact_ref: Some(
                            "oci://registry.example/pika@sha256:deadbeef".to_string(),
                        ),
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        assert!(out.result.is_some());
    }

    #[tokio::test]
    async fn v2_get_build_is_available_without_phase_gates() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            test_build_policy(vec![]),
        );
        let requester = Keys::generate().public_key();

        let submit = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-submit-get",
                    "idem-build-submit-get",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: None,
                        artifact_ref: Some("oci://registry.example/pika@sha256:abc123".to_string()),
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        let build_id = submit
            .result
            .expect("submit build should succeed")
            .payload
            .get("build")
            .and_then(|build| build.get("build_id"))
            .and_then(Value::as_str)
            .expect("build id")
            .to_string();

        let get = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-get-no-flags",
                    "idem-build-get-no-flags",
                    AgentControlCommand::GetBuild(GetBuildCommand { build_id }),
                ),
            )
            .await;
        assert!(get.result.is_some());
    }

    #[tokio::test]
    async fn v2_cancel_build_is_available_without_phase_gates() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: true,
            },
            test_build_policy(vec!["git+https://github.com/"]),
        );
        let requester = Keys::generate().public_key();

        let submit = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-submit-cancel",
                    "idem-build-submit-cancel",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: Some("git+https://github.com/example/repo".to_string()),
                        artifact_ref: None,
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        let build_id = submit
            .result
            .expect("submit build should succeed")
            .payload
            .get("build")
            .and_then(|build| build.get("build_id"))
            .and_then(Value::as_str)
            .expect("build id")
            .to_string();

        let canceled = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-cancel-no-flags",
                    "idem-build-cancel-no-flags",
                    AgentControlCommand::CancelBuild(CancelBuildCommand { build_id }),
                ),
            )
            .await;
        assert!(canceled.result.is_some());
    }

    #[tokio::test]
    async fn v2_submit_build_and_get_build_succeeds_for_immutable_artifact() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            test_build_policy(vec![]),
        );
        let requester = Keys::generate().public_key();
        let submit = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-submit",
                    "idem-build-submit",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: None,
                        artifact_ref: Some(
                            "oci://registry.example/pika@sha256:001122334455".to_string(),
                        ),
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        let submit_result = submit.result.expect("submit build should succeed");
        let build = submit_result
            .payload
            .get("build")
            .and_then(Value::as_object)
            .expect("build payload");
        assert_eq!(
            build.get("phase").and_then(Value::as_str),
            Some("succeeded")
        );
        let build_id = build
            .get("build_id")
            .and_then(Value::as_str)
            .expect("build id")
            .to_string();

        let get = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-get",
                    "idem-build-get",
                    AgentControlCommand::GetBuild(GetBuildCommand { build_id }),
                ),
            )
            .await;
        let get_result = get.result.expect("get build should succeed");
        assert_eq!(
            get_result
                .payload
                .get("build")
                .and_then(|build| build.get("phase"))
                .and_then(Value::as_str),
            Some("succeeded")
        );
    }

    #[tokio::test]
    async fn v2_submit_build_source_requires_advanced_mode_and_allowlist() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let requester = Keys::generate().public_key();

        let service_no_advanced = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter.clone(),
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            test_build_policy(vec!["git+https://github.com/"]),
        );
        let out = service_no_advanced
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-src-1",
                    "idem-build-src-1",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: Some("git+https://github.com/example/repo".to_string()),
                        artifact_ref: None,
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        assert_eq!(
            out.error.expect("advanced disabled").code,
            "v2_advanced_workload_disabled"
        );

        let service_no_prefixes = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: true,
            },
            test_build_policy(vec![]),
        );
        let out = service_no_prefixes
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-src-2",
                    "idem-build-src-2",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: Some("git+https://github.com/example/repo".to_string()),
                        artifact_ref: None,
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        assert_eq!(
            out.error.expect("source prefixes missing").code,
            "build_source_disabled"
        );
    }

    #[tokio::test]
    async fn v2_submit_build_source_allowlist_rejects_prefix_confusion() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: true,
            },
            test_build_policy(vec!["git+https://github.com/acme/repo"]),
        );
        let requester = Keys::generate().public_key();

        for blocked in [
            "git+https://github.com.evil.com/acme/repo",
            "git+https://github.com/acme/repo-evil",
            "git+https://github.com/acme/repo/../evil",
        ] {
            let out = service
                .handle_command(
                    &requester.to_hex(),
                    requester,
                    request_with(
                        "req-build-prefix-confusion",
                        "idem-build-prefix-confusion",
                        AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                            build_kind: BuildKind::Oci,
                            source_ref: Some(blocked.to_string()),
                            artifact_ref: None,
                            timeout_sec: Some(120),
                            context_bytes: Some(64),
                        }),
                    ),
                )
                .await;
            assert!(out.result.is_none(), "{blocked} should be blocked");
            assert_eq!(
                out.error.expect("blocked source ref").code,
                "build_source_not_allowed"
            );
        }

        let allowed = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-prefix-allowed",
                    "idem-build-prefix-allowed",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: Some("git+https://github.com/acme/repo/subdir".to_string()),
                        artifact_ref: None,
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        assert!(allowed.result.is_some());
    }

    #[tokio::test]
    async fn v2_build_submission_rate_limit_enforced() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let mut policy = test_build_policy(vec![]);
        policy.max_submissions_per_hour = 1;
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            policy,
        );
        let requester = Keys::generate().public_key();

        let first = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-1",
                    "idem-build-1",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: None,
                        artifact_ref: Some("oci://r/p@sha256:1111".to_string()),
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        assert!(first.result.is_some());

        let second = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-2",
                    "idem-build-2",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: None,
                        artifact_ref: Some("oci://r/p@sha256:2222".to_string()),
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        assert!(second.result.is_none());
        assert_eq!(
            second.error.expect("second submit should rate limit").code,
            "build_rate_limited"
        );
    }

    #[tokio::test]
    async fn v2_cancel_build_transitions_pending_build() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: true,
            },
            test_build_policy(vec!["git+https://github.com/"]),
        );
        let requester = Keys::generate().public_key();

        let submit = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-pending",
                    "idem-build-pending",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: Some("git+https://github.com/example/repo".to_string()),
                        artifact_ref: None,
                        timeout_sec: Some(300),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        let submit_result = submit.result.expect("pending build submit should succeed");
        let build_id = submit_result
            .payload
            .get("build")
            .and_then(|build| build.get("build_id"))
            .and_then(Value::as_str)
            .expect("build id")
            .to_string();

        let canceled = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-cancel",
                    "idem-build-cancel",
                    AgentControlCommand::CancelBuild(CancelBuildCommand { build_id }),
                ),
            )
            .await;
        let result = canceled.result.expect("cancel should succeed");
        assert_eq!(
            result
                .payload
                .get("build")
                .and_then(|build| build.get("phase"))
                .and_then(Value::as_str),
            Some("canceled")
        );
    }

    #[tokio::test]
    async fn v2_provision_with_build_id_attaches_artifact_metadata() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            test_build_policy(vec![]),
        );
        let requester = Keys::generate().public_key();

        let submit = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-build-submit",
                    "idem-build-submit",
                    AgentControlCommand::SubmitBuild(SubmitBuildCommand {
                        build_kind: BuildKind::Oci,
                        source_ref: None,
                        artifact_ref: Some("oci://r/p@sha256:123456".to_string()),
                        timeout_sec: Some(120),
                        context_bytes: Some(64),
                    }),
                ),
            )
            .await;
        let build_id = submit
            .result
            .expect("build submit")
            .payload
            .get("build")
            .and_then(|build| build.get("build_id"))
            .and_then(Value::as_str)
            .expect("build id")
            .to_string();

        let provision = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-provision-build",
                    "idem-provision-build",
                    AgentControlCommand::Provision(ProvisionCommand {
                        provider: ProviderKind::Fly,
                        protocol: ProtocolKind::Acp,
                        name: None,
                        runtime_class: None,
                        relay_urls: vec![],
                        keep: false,
                        bot_secret_key_hex: None,
                        build_id: Some(build_id),
                        artifact_ref: None,
                        advanced_workload_json: None,
                        microvm: None,
                    }),
                ),
            )
            .await;
        let runtime = provision.result.expect("provision should succeed").runtime;
        assert_eq!(
            runtime
                .metadata
                .get("artifact_ref")
                .and_then(Value::as_str)
                .unwrap_or(""),
            "oci://r/p@sha256:123456"
        );
        assert!(runtime.metadata.get("build_id").is_some());
    }

    #[tokio::test]
    async fn unauthorized_get_build_does_not_mutate_build_state() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let now = 1_700_200_000u64;
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: true,
            },
            test_build_policy(vec!["git+https://github.com/acme/repo"]),
        );
        let owner = Keys::generate().public_key();
        let unauthorized = Keys::generate().public_key();
        let build_id = "build-owner-only".to_string();
        {
            let mut state = service.state.write().await;
            state.builds.insert(
                build_id.clone(),
                BuildRecord {
                    build_id: build_id.clone(),
                    owner_pubkey_hex: owner.to_hex(),
                    build_kind: BuildKind::Oci,
                    phase: BuildPhase::FetchingSource,
                    source_ref: Some("git+https://github.com/acme/repo".to_string()),
                    artifact_ref: None,
                    created_at: now.saturating_sub(120),
                    updated_at: now.saturating_sub(120),
                    deadline_at: now.saturating_add(600),
                    ready_at: Some(now.saturating_sub(1)),
                    context_bytes: 64,
                    timeout_sec: 300,
                    error_code: None,
                    error_detail: None,
                    canceled_at: None,
                },
            );
        }

        let out = service
            .handle_command(
                &unauthorized.to_hex(),
                unauthorized,
                request_with(
                    "req-unauthorized-get-build",
                    "idem-unauthorized-get-build",
                    AgentControlCommand::GetBuild(GetBuildCommand {
                        build_id: build_id.clone(),
                    }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        assert_eq!(out.error.expect("unauthorized get").code, "build_not_found");

        let state = service.state.read().await;
        let build = state.builds.get(&build_id).expect("build exists");
        assert_eq!(build.phase, BuildPhase::FetchingSource);
        assert_eq!(build.artifact_ref, None);
        assert_eq!(build.updated_at, now.saturating_sub(120));
    }

    #[tokio::test]
    async fn unauthorized_provision_build_id_does_not_mutate_build_state() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let now = 1_700_200_100u64;
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: true,
            },
            test_build_policy(vec!["git+https://github.com/acme/repo"]),
        );
        let owner = Keys::generate().public_key();
        let unauthorized = Keys::generate().public_key();
        let build_id = "build-provision-owner-only".to_string();
        {
            let mut state = service.state.write().await;
            state.builds.insert(
                build_id.clone(),
                BuildRecord {
                    build_id: build_id.clone(),
                    owner_pubkey_hex: owner.to_hex(),
                    build_kind: BuildKind::Oci,
                    phase: BuildPhase::FetchingSource,
                    source_ref: Some("git+https://github.com/acme/repo".to_string()),
                    artifact_ref: None,
                    created_at: now.saturating_sub(120),
                    updated_at: now.saturating_sub(120),
                    deadline_at: now.saturating_add(600),
                    ready_at: Some(now.saturating_sub(1)),
                    context_bytes: 64,
                    timeout_sec: 300,
                    error_code: None,
                    error_detail: None,
                    canceled_at: None,
                },
            );
        }

        let out = service
            .handle_command(
                &unauthorized.to_hex(),
                unauthorized,
                request_with(
                    "req-unauthorized-provision-build",
                    "idem-unauthorized-provision-build",
                    AgentControlCommand::Provision(ProvisionCommand {
                        provider: ProviderKind::Fly,
                        protocol: ProtocolKind::Acp,
                        name: None,
                        runtime_class: None,
                        relay_urls: vec![],
                        keep: false,
                        bot_secret_key_hex: None,
                        build_id: Some(build_id.clone()),
                        artifact_ref: None,
                        advanced_workload_json: None,
                        microvm: None,
                    }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        assert_eq!(
            out.error.expect("unauthorized provision").code,
            "build_not_found"
        );

        let state = service.state.read().await;
        let build = state.builds.get(&build_id).expect("build exists");
        assert_eq!(build.phase, BuildPhase::FetchingSource);
        assert_eq!(build.artifact_ref, None);
        assert_eq!(build.updated_at, now.saturating_sub(120));
    }

    #[tokio::test]
    async fn artifact_gc_removes_only_unreferenced_expired_entries() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_flags_and_build_policy(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
            test_build_policy(vec![]),
        );
        let now = 1_700_100_000u64;
        {
            let mut state = service.state.write().await;
            state.artifacts.insert(
                "oci://unused@sha256:1".to_string(),
                ArtifactRecord {
                    artifact_ref: "oci://unused@sha256:1".to_string(),
                    build_kind: BuildKind::Oci,
                    owner_pubkey_hex: "owner".to_string(),
                    created_at: now.saturating_sub(500),
                    last_used_at: now.saturating_sub(500),
                    expires_at: now.saturating_sub(1),
                    source_build_id: None,
                },
            );
            state.artifacts.insert(
                "oci://used@sha256:2".to_string(),
                ArtifactRecord {
                    artifact_ref: "oci://used@sha256:2".to_string(),
                    build_kind: BuildKind::Oci,
                    owner_pubkey_hex: "owner".to_string(),
                    created_at: now.saturating_sub(500),
                    last_used_at: now.saturating_sub(500),
                    expires_at: now.saturating_sub(1),
                    source_build_id: None,
                },
            );
            state.runtimes.insert(
                "runtime-used-artifact".to_string(),
                RuntimeRecord {
                    owner_pubkey_hex: "owner".to_string(),
                    descriptor: RuntimeDescriptor {
                        runtime_id: "runtime-used-artifact".to_string(),
                        provider: ProviderKind::Fly,
                        lifecycle_phase: RuntimeLifecyclePhase::Ready,
                        runtime_class: Some("mock".to_string()),
                        region: Some("local".to_string()),
                        capacity: Value::Null,
                        policy_constraints: Value::Null,
                        protocol_compatibility: vec![ProtocolKind::Acp],
                        bot_pubkey: Some("ab".repeat(32)),
                        metadata: json!({
                            "artifact_ref": "oci://used@sha256:2",
                        }),
                    },
                    provider_handle: ProviderHandle::Fly {
                        machine_id: "machine-1".to_string(),
                        volume_id: "volume-1".to_string(),
                        app_name: "app".to_string(),
                    },
                    created_at: now.saturating_sub(100),
                    expires_at: now.saturating_add(100),
                    teardown_retry: None,
                },
            );
        }
        let removed = service
            .garbage_collect_artifacts_at(now)
            .await
            .expect("gc succeeds");
        assert_eq!(removed, 1);
        let state = service.state.read().await;
        assert!(!state.artifacts.contains_key("oci://unused@sha256:1"));
        assert!(state.artifacts.contains_key("oci://used@sha256:2"));
    }

    #[tokio::test]
    async fn v2_get_capabilities_is_available_without_phase_gates() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );
        let requester = Keys::generate().public_key();
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-capabilities",
                    "idem-capabilities",
                    AgentControlCommand::GetCapabilities(GetCapabilitiesCommand::default()),
                ),
            )
            .await;
        assert!(out.result.is_some());
    }

    #[tokio::test]
    async fn v2_get_capabilities_returns_provider_payload() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );
        let requester = Keys::generate().public_key();
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-capabilities",
                    "idem-capabilities",
                    AgentControlCommand::GetCapabilities(GetCapabilitiesCommand::default()),
                ),
            )
            .await;
        let result = out.result.expect("capabilities should succeed");
        let providers = result
            .payload
            .get("providers")
            .and_then(Value::as_array)
            .expect("providers array");
        assert_eq!(providers.len(), 2);
    }

    #[tokio::test]
    async fn v2_get_capabilities_requires_provision_allowlist() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let allowed = Keys::generate().public_key();
        let denied = Keys::generate().public_key();
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::Allowlist(HashSet::from([allowed.to_hex()])),
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );

        let out = service
            .handle_command(
                &denied.to_hex(),
                denied,
                request_with(
                    "req-capabilities",
                    "idem-capabilities",
                    AgentControlCommand::GetCapabilities(GetCapabilitiesCommand::default()),
                ),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("capabilities should enforce allowlist");
        assert_eq!(err.code, "provision_unauthorized");
    }

    #[tokio::test]
    async fn v2_resolve_distribution_is_available_without_phase_gates() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );
        let requester = Keys::generate().public_key();
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-resolve",
                    "idem-resolve",
                    AgentControlCommand::ResolveDistribution(ResolveDistributionCommand {
                        distribution_ref: "agent.default".to_string(),
                        preset: "small".to_string(),
                        overrides_json: None,
                    }),
                ),
            )
            .await;
        assert!(out.result.is_some());
    }

    #[tokio::test]
    async fn v2_resolve_distribution_returns_resolved_provision() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );
        let requester = Keys::generate().public_key();
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-resolve",
                    "idem-resolve",
                    AgentControlCommand::ResolveDistribution(ResolveDistributionCommand {
                        distribution_ref: "agent.default".to_string(),
                        preset: "small".to_string(),
                        overrides_json: Some("{\"ttl_sec\":120}".to_string()),
                    }),
                ),
            )
            .await;
        let result = out.result.expect("distribution resolve should succeed");
        assert_eq!(
            result
                .payload
                .get("resolved_provision")
                .and_then(|provision| provision.get("requested_ttl_sec"))
                .and_then(Value::as_u64),
            Some(120)
        );
    }

    #[tokio::test]
    async fn v2_resolve_distribution_rejects_disallowed_override() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::AllowAll,
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );
        let requester = Keys::generate().public_key();
        let out = service
            .handle_command(
                &requester.to_hex(),
                requester,
                request_with(
                    "req-resolve",
                    "idem-resolve",
                    AgentControlCommand::ResolveDistribution(ResolveDistributionCommand {
                        distribution_ref: "agent.default".to_string(),
                        preset: "small".to_string(),
                        overrides_json: Some("{\"runtime_class\":\"forbidden\"}".to_string()),
                    }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("disallowed override should fail");
        assert_eq!(err.code, "distribution_override_not_allowed");
    }

    #[tokio::test]
    async fn v2_resolve_distribution_requires_provision_allowlist() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let adapter = std::sync::Arc::new(MockAdapter {
            calls: calls.clone(),
        });
        let allowed = Keys::generate().public_key();
        let denied = Keys::generate().public_key();
        let service = AgentControlService::with_adapters_policy_and_flags(
            adapter.clone(),
            adapter,
            ProvisionPolicy::Allowlist(HashSet::from([allowed.to_hex()])),
            DEFAULT_IDEMPOTENCY_MAX_ENTRIES,
            V2RolloutFlags {
                advanced_workload_enabled: false,
            },
        );
        let out = service
            .handle_command(
                &denied.to_hex(),
                denied,
                request_with(
                    "req-resolve",
                    "idem-resolve",
                    AgentControlCommand::ResolveDistribution(ResolveDistributionCommand {
                        distribution_ref: "agent.default".to_string(),
                        preset: "small".to_string(),
                        overrides_json: None,
                    }),
                ),
            )
            .await;
        assert!(out.result.is_none());
        let err = out.error.expect("resolve should enforce allowlist");
        assert_eq!(err.code, "provision_unauthorized");
    }
}
