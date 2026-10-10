-- **A write, through two mechanisms at once.** The bogus qualifier sits in a `HAVING`
-- inside an `IN (…)` subquery of a `DELETE`: the aggregate path's post-projection
-- resolver answered it from the subquery's own `ikey`, the `HAVING` was true, and the
-- `DELETE` removed a row. 18.6 raises `42P01` before any of it.
--
-- It needed both halves of the plan-time gate to close — the sibling class's (an
-- aggregate scope has no relation identity) *and* the chain-completeness signal (the
-- subquery is a nested level, so its scope is provisional until the enclosing level's
-- re-run). Either alone leaves this statement writing.
-- @setup
CREATE TABLE sab (ikey INT, tag TEXT);
INSERT INTO sab VALUES (7, 'z'), (8, 'y');

-- @expect-error: missing FROM-clause entry for table "bogusrel" (42P01)
-- @test
DELETE FROM sab WHERE ikey IN (SELECT ikey FROM sab GROUP BY ikey HAVING bogusrel.ikey = 7);
