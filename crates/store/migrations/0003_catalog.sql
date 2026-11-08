CREATE TABLE artist (
    key INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
) STRICT;

-- `artist_key` is the first artist credited. `kind` is 0 for an album, 1 for a single or
-- EP, 2 for a compilation.
ALTER TABLE track DROP COLUMN album;
ALTER TABLE track ADD COLUMN artist_key INTEGER;
ALTER TABLE album ADD COLUMN artist_key INTEGER;
ALTER TABLE album ADD COLUMN year INTEGER;
ALTER TABLE album ADD COLUMN kind INTEGER NOT NULL DEFAULT 0;

-- Read the library again, so its tracks and albums gain their artist.
DELETE FROM list_item WHERE list_id IN (-1, -2);
DELETE FROM sync_cursor;
