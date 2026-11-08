-- User settings by name, each a JSON value.
CREATE TABLE settings (
    name TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
