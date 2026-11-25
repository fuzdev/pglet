-- A read-only `WITH` query that reads the data-modifying one, unread by the statement,
-- leaves the data-modifying one unread too: 18.6 runs it after the statement, so it is
-- refused (`0A000`; 18.6 answers).
-- @setup
CREATE TABLE dmx (id INT, v INT);
CREATE TABLE dmx_d (tid INT);
-- @no-profile-lock: refused while the statement is parsed, before any index is chosen
-- @expect-error: a writing body that does not read the data-modifying WITH
-- @test
WITH c AS (INSERT INTO dmx VALUES (1, 1) RETURNING id), r AS (SELECT id FROM c)
INSERT INTO dmx_d (tid) VALUES (5);
