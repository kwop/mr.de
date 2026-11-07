BEGIN;
CREATE TABLE IF NOT EXISTS resolver_threads (
    id TEXT PRIMARY KEY, instance TEXT NOT NULL,
    generation INTEGER NOT NULL, sequence INTEGER NOT NULL,
    updated_at TEXT NOT NULL, payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS resolver_threads_instance ON resolver_threads(instance, updated_at);
PRAGMA user_version = 2;
COMMIT;
