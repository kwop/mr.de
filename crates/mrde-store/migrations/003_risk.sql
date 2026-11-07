BEGIN;
CREATE TABLE IF NOT EXISTS risk_profiles (
    namespace TEXT NOT NULL,
    project_id INTEGER NOT NULL,
    iid INTEGER NOT NULL,
    base_sha TEXT NOT NULL,
    head_sha TEXT NOT NULL,
    config_key TEXT NOT NULL,
    computed_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY(namespace, project_id, iid, base_sha, head_sha, config_key)
);
CREATE INDEX IF NOT EXISTS risk_latest ON risk_profiles(namespace, project_id, iid, head_sha, computed_at DESC);
PRAGMA user_version = 3;
COMMIT;
