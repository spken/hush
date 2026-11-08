-- When each playlist last changed, in Unix milliseconds, as Spotify's playlist list says;
-- NULL while unknown.
ALTER TABLE playlist ADD COLUMN modified_at INTEGER;

-- The playlists pinned to the top of the library. The one pinned last has the highest
-- `seq`.
CREATE TABLE pinned (
    key INTEGER PRIMARY KEY,
    seq INTEGER NOT NULL
) STRICT;
