-- A `PRIMARY KEY`'s and a `UNIQUE`'s index carries the constraint's name, so it is the
-- same cut name — read here from the index side, beside an unnamed plain index over the
-- same column, whose label is `idx`.
-- @setup
CREATE TABLE cac8_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo INT UNIQUE, id INT PRIMARY KEY, UNIQUE (id, cooooooooooooooooooooooooooooooooooooooo));
ALTER TABLE cac8_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD UNIQUE (cooooooooooooooooooooooooooooooooooooooo);
CREATE INDEX ON cac8_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo);
-- @test
SELECT indexname::text AS indexname, length(indexname::text) AS len FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'cac8_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt' ORDER BY indexname;
