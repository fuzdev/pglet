-- A `DO` block after a `WITH` list is a syntax error.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: syntax error at DO (42601)
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3) DO $$ BEGIN INSERT INTO s VALUES (77); END $$;
