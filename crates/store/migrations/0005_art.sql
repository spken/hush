-- Each entity's picture as `width url` lines, narrowest first; '' while unknown. A track's
-- is its album's cover.
ALTER TABLE track ADD COLUMN art TEXT NOT NULL DEFAULT '';
ALTER TABLE album ADD COLUMN art TEXT NOT NULL DEFAULT '';
ALTER TABLE artist ADD COLUMN art TEXT NOT NULL DEFAULT '';
ALTER TABLE playlist ADD COLUMN art TEXT NOT NULL DEFAULT '';

-- Read the library and catalog lists again, so their entities gain their pictures.
DELETE FROM list_item WHERE list_id IN (-1, -2);
DELETE FROM sync_cursor;
