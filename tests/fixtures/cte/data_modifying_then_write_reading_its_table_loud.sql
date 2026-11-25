-- A body reading the table the data-modifying `WITH` writes would see the new rows here
-- and the statement's snapshot there, so it is refused (`0A000`; 18.6 answers).
-- @setup
CREATE TABLE dmr (id SERIAL PRIMARY KEY, kind TEXT);
CREATE TABLE dmr_g (cell_id INT, kind TEXT);
-- @no-profile-lock: refused while the statement is parsed, before any index is chosen
-- @expect-error: a writing body naming the table the WITH writes
-- @test
WITH c AS (INSERT INTO dmr (kind) VALUES ('q') RETURNING id)
INSERT INTO dmr_g SELECT id, (SELECT kind FROM dmr WHERE dmr.id = c.id) FROM c;
