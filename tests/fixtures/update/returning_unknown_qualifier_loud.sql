-- A bogus table qualifier in a RETURNING item is a `42P01` (missing FROM-clause
-- entry). A loose match would bind `wrong.v` to the target's `v` and return the
-- post-image; Postgres rejects the unknown qualifier (note: PG18 accepts the
-- special `OLD`/`NEW` correlation names, which pglet does not model — those stay
-- fail-loud too, the documented pglet-stricter posture). A valid target-qualified
-- `RETURNING` (`t.v`, or the statement alias) still resolves — see the engine
-- tests `update_with_table_alias_resolves_and_returns` /
-- `update_from_joins_source_and_updates_matches`.
-- @setup
CREATE TABLE tri_q (v INT);
INSERT INTO tri_q VALUES (1);

-- @test
-- @expect-error: missing FROM-clause entry for table "wrong" (42P01)
UPDATE tri_q SET v = 2 RETURNING wrong.v;
