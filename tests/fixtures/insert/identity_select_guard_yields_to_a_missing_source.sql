-- The `INSERT … SELECT` twin of `update/identity_guard_yields_to_a_missing_source.sql`.
-- `enforce_identity_select` stands in for `rewriteTargetListIU`'s refusal, which 18.6
-- raises after all of parse analysis — and the source relation's `42P01` comes before
-- parse analysis, so the guard cannot be reached at all on this statement.
--
-- Its control is the `OVERRIDING SYSTEM VALUE` spelling, which switches the guard off
-- and was `42P01` on both engines throughout — the answer that identifies the guard,
-- rather than the target list, as what used to pre-empt the missing relation.
-- @setup
CREATE TABLE acis (id INT GENERATED ALWAYS AS IDENTITY, v INT);

-- @expect-error: relation "acis_nosuch" does not exist (42P01)
-- @test
INSERT INTO acis (id, v) SELECT 5, 1 FROM acis_nosuch;
