-- One shard database. Thumbnail generations are immutable rows. The main database says which
-- generation of a name is current.
CREATE TABLE blobs (
  repo TEXT NOT NULL,
  schema INTEGER NOT NULL,
  name TEXT NOT NULL,
  gen TEXT NOT NULL,
  bytes BLOB NOT NULL,
  -- Insert time in milliseconds since the epoch. The sweep leaves a row alone for an hour, so a
  -- PUT that is still running never loses its rows.
  created INTEGER NOT NULL,
  PRIMARY KEY (repo, schema, name, gen)
);

-- Bytes held by this shard. A PUT reserves here before it inserts. A shard refuses at 300 MiB of
-- payload, because SQLite pages add up to 1.4 times the payload for thumbnails of about 6 KB.
-- `sweep_after` is the row id after which the next sweep run looks.
CREATE TABLE usage (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  bytes INTEGER NOT NULL,
  sweep_after INTEGER NOT NULL DEFAULT 0
);

INSERT INTO usage (id, bytes) VALUES (1, 0);
