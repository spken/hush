-- Full-text indexes over the entity tables' text, for library search. Triggers keep them in
-- step with inserts and with updates that change the text; entity rows are never deleted.
CREATE VIRTUAL TABLE track_fts USING fts5 (
    name, artists, content = 'track', content_rowid = 'key',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE album_fts USING fts5 (
    name, artists, content = 'album', content_rowid = 'key',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE artist_fts USING fts5 (
    name, content = 'artist', content_rowid = 'key',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE VIRTUAL TABLE playlist_fts USING fts5 (
    name, owner, content = 'playlist', content_rowid = 'key',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER track_fts_insert AFTER INSERT ON track BEGIN
    INSERT INTO track_fts (rowid, name, artists) VALUES (new.key, new.name, new.artists);
END;
CREATE TRIGGER track_fts_update AFTER UPDATE OF name, artists ON track
WHEN old.name IS NOT new.name OR old.artists IS NOT new.artists BEGIN
    INSERT INTO track_fts (track_fts, rowid, name, artists)
    VALUES ('delete', old.key, old.name, old.artists);
    INSERT INTO track_fts (rowid, name, artists) VALUES (new.key, new.name, new.artists);
END;

CREATE TRIGGER album_fts_insert AFTER INSERT ON album BEGIN
    INSERT INTO album_fts (rowid, name, artists) VALUES (new.key, new.name, new.artists);
END;
CREATE TRIGGER album_fts_update AFTER UPDATE OF name, artists ON album
WHEN old.name IS NOT new.name OR old.artists IS NOT new.artists BEGIN
    INSERT INTO album_fts (album_fts, rowid, name, artists)
    VALUES ('delete', old.key, old.name, old.artists);
    INSERT INTO album_fts (rowid, name, artists) VALUES (new.key, new.name, new.artists);
END;

CREATE TRIGGER artist_fts_insert AFTER INSERT ON artist BEGIN
    INSERT INTO artist_fts (rowid, name) VALUES (new.key, new.name);
END;
CREATE TRIGGER artist_fts_update AFTER UPDATE OF name ON artist
WHEN old.name IS NOT new.name BEGIN
    INSERT INTO artist_fts (artist_fts, rowid, name) VALUES ('delete', old.key, old.name);
    INSERT INTO artist_fts (rowid, name) VALUES (new.key, new.name);
END;

CREATE TRIGGER playlist_fts_insert AFTER INSERT ON playlist BEGIN
    INSERT INTO playlist_fts (rowid, name, owner) VALUES (new.key, new.name, new.owner);
END;
CREATE TRIGGER playlist_fts_update AFTER UPDATE OF name, owner ON playlist
WHEN old.name IS NOT new.name OR old.owner IS NOT new.owner BEGIN
    INSERT INTO playlist_fts (playlist_fts, rowid, name, owner)
    VALUES ('delete', old.key, old.name, old.owner);
    INSERT INTO playlist_fts (rowid, name, owner) VALUES (new.key, new.name, new.owner);
END;

INSERT INTO track_fts (track_fts) VALUES ('rebuild');
INSERT INTO album_fts (album_fts) VALUES ('rebuild');
INSERT INTO artist_fts (artist_fts) VALUES ('rebuild');
INSERT INTO playlist_fts (playlist_fts) VALUES ('rebuild');
