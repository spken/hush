-- When the user last played each playlist, from Spotify's listening history, as ISO 8601
-- UTC text, which sorts in time order; '' while unknown.
ALTER TABLE playlist ADD COLUMN played_at TEXT NOT NULL DEFAULT '';
