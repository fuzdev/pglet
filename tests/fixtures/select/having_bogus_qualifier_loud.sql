-- A bogus qualifier in `HAVING` **returned the row**. The aggregate path resolves through
-- a post-projection scope with no relation identity, whose last tier drops the qualifier
-- and matches the bare name — so `bogusrel.tag` read `hbq`'s own `tag` and the predicate
-- was true.
--
-- The plan-time gate closes it without touching that resolver: the planner knows the FROM
-- namespace even where the executor's post-projection node does not. Its neighbours are
-- the window and `ORDER BY` spellings, each reaching the same resolver from a different
-- clause.
-- @setup
CREATE TABLE hbq (ikey INT, tag TEXT);
INSERT INTO hbq VALUES (7, 'z');

-- @expect-error: missing FROM-clause entry for table "bogusrel" (42P01)
-- @test
SELECT ikey FROM hbq GROUP BY ikey HAVING bogusrel.tag = 'z';
