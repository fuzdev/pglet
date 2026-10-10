-- The `SET` right-hand side is the write path's most dangerous copy of this bug:
-- an offset-less `'v=' || a` is **persisted**, so the silent instant shift lands
-- durably in the table rather than in one query's output. The test selects the
-- stored column back, so it asserts what was written, not what was computed.
-- @setup
CREATE TABLE uctz (id INT, a TIMESTAMPTZ, s TEXT);
INSERT INTO uctz VALUES (1, '2026-07-18 12:30:45+00', NULL);
UPDATE uctz SET s = 'v=' || a WHERE id = 1;
-- @test
SELECT id, s FROM uctz ORDER BY id;
