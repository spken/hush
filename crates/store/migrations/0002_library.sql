CREATE TABLE track (
    key INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    artists TEXT NOT NULL,
    album TEXT NOT NULL,
    duration_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE album (
    key INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    artists TEXT NOT NULL
) STRICT;

CREATE TABLE playlist (
    key INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    owner TEXT NOT NULL,
    collaborative INTEGER NOT NULL,
    tracks INTEGER NOT NULL,
    snapshot TEXT NOT NULL
) STRICT;

-- The order of every list. `list_id` is -1 for Liked Songs, -2 for saved albums, -3 for
-- the playlists, and a playlist's key for its tracks.
CREATE TABLE list_item (
    list_id INTEGER NOT NULL,
    pos INTEGER NOT NULL,
    entity_key INTEGER NOT NULL,
    added_at TEXT
) STRICT;

CREATE INDEX list_item_pos ON list_item (list_id, pos);
CREATE INDEX list_item_entity ON list_item (list_id, entity_key);

-- When each list was last synced and, for a playlist, the snapshot its tracks came from.
CREATE TABLE sync_cursor (
    list_id INTEGER PRIMARY KEY,
    synced_at INTEGER NOT NULL,
    snapshot TEXT
) STRICT;
