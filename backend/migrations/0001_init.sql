CREATE TABLE events (id TEXT NOT NULL, repo TEXT NOT NULL, sha TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind IN ('view','send')), ts INTEGER NOT NULL, PRIMARY KEY (repo, id));
CREATE INDEX events_ts ON events(ts);
CREATE TABLE thumb_rev (repo TEXT NOT NULL, schema INTEGER NOT NULL, rev INTEGER NOT NULL, PRIMARY KEY (repo, schema));
CREATE TABLE thumbs (repo TEXT NOT NULL, schema INTEGER NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL, rev INTEGER NOT NULL, PRIMARY KEY (repo, schema, name));
CREATE INDEX thumbs_cursor ON thumbs(repo, schema, rev, name);
