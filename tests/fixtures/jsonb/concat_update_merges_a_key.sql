-- The merge as an update: one key replaced, the rest kept.
-- @setup
CREATE TABLE jcu (id INT, data JSONB);
INSERT INTO jcu VALUES (1, '{"topic": "old", "n": 1}'), (2, '{"n": 2}');
-- @test
UPDATE jcu SET data = data || '{"topic": "kept"}'::jsonb WHERE id = 1 RETURNING id, data;
