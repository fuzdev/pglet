-- A projection naming a column that does not exist is `42703`, reported ahead of an
-- out-of-range `GROUP BY` ordinal. Postgres resolves the select list before the SQL92
-- clause rules, so `SELECT bogus … GROUP BY 99` is the missing column, not the bad
-- position. GROUP BY key resolution runs after the projection's names are bound for
-- the same reason.
--
-- The table has a single column; the two-column spelling answers `42703` on both
-- engines as well, so the arity decides nothing here.
-- @setup
CREATE TABLE gbmcp (c INT);
INSERT INTO gbmcp VALUES (1), (2);
-- @test
-- @expect-error: column "bogus" does not exist (42703)
SELECT bogus FROM gbmcp GROUP BY 99;
