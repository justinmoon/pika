-- Restore the one-active-agent-per-owner unique index.
CREATE UNIQUE INDEX agent_instances_owner_active_idx
    ON agent_instances (owner_npub)
    WHERE phase IN ('creating', 'ready');

ALTER TABLE agent_allowlist DROP COLUMN IF EXISTS max_agents;
