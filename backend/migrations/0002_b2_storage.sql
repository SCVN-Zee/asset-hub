CREATE TABLE objects (repo TEXT NOT NULL, schema INTEGER NOT NULL, name TEXT NOT NULL, etag TEXT NOT NULL, key TEXT NOT NULL, version_id TEXT NOT NULL, size INTEGER NOT NULL, PRIMARY KEY (repo, schema, name));
CREATE TABLE storage (id INTEGER PRIMARY KEY CHECK (id = 1), reserved_bytes INTEGER NOT NULL);
INSERT INTO storage (id, reserved_bytes) VALUES (1, 0);
ALTER TABLE thumbs ADD COLUMN version_id TEXT NOT NULL DEFAULT '';
