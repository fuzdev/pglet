-- The other direction, and the silent-wrong one: a standalone `CREATE UNIQUE INDEX
-- gbn_pkey` is an index, not a primary key, so `name` is *not* functionally dependent
-- and the query is 42803. A `<table>_pkey` name lookup accepted it and returned rows.
-- @setup
CREATE TABLE gbn (id INT, name TEXT);
CREATE UNIQUE INDEX gbn_pkey ON gbn (id);
INSERT INTO gbn VALUES (1, 'a');

-- @expect-error: column gbn.name must appear in the GROUP BY clause
-- @profile-lock
-- @test
SELECT id, name FROM gbn GROUP BY id ORDER BY id;
