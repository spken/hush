CREATE TABLE http_cache (
    url TEXT PRIMARY KEY NOT NULL,
    etag TEXT,
    body BLOB NOT NULL,
    expires_at INTEGER NOT NULL
) STRICT;
