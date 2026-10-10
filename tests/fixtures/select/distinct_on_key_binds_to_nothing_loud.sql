-- @setup
CREATE TABLE djk (k INT, v INT);
INSERT INTO djk VALUES (1, 10), (1, 20), (2, 30);

-- A `DISTINCT ON` key the projection cannot answer rides the projecting node as a junk
-- column, which is evaluated only when a row is — so a key binding to nothing is judged
-- by nobody on a statement matching none. The refusal that used to double as this check
-- was `project_distinct_on_key`'s unconditional `0A000`; with rows present this
-- statement is `42703` either way, which is what identifies the hole as the no-row one.
-- @expect-error: a DISTINCT ON key that binds to nothing is 42703 even with no row
-- @test
SELECT DISTINCT ON (bogus) v FROM djk WHERE 1 = 0 ORDER BY bogus;
