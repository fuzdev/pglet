-- SQL 3-valued logic: a CHECK that evaluates to NULL (unknown) PASSES. `level`
-- is nullable, so `level IN ('viewer','editor')` over a NULL `level` is NULL,
-- and the row is accepted — both an explicit NULL and an omitted column.
-- @setup
CREATE TABLE ck (id INT, level TEXT CHECK (level IN ('viewer', 'editor')));
INSERT INTO ck (id, level) VALUES (1, NULL);
INSERT INTO ck (id) VALUES (2);
INSERT INTO ck (id, level) VALUES (3, 'editor');

-- @test
SELECT id, level FROM ck ORDER BY id;
