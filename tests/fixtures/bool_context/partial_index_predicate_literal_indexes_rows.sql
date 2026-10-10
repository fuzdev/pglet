-- A partial index's predicate is decided in two crates by two implementations —
-- storage's `row_in_index` (is the row *stored*) and the executor's
-- `prepared_unique_index_key` (is it a *uniqueness candidate*) — and nothing
-- compares them. An unresolved literal predicate that resolves TRUE must make both
-- say yes: the row is indexed **and** enforced, so the duplicate is refused.
-- @setup
CREATE TABLE bcpi (a INT, b INT);
CREATE UNIQUE INDEX bcpi_ix ON bcpi (a) WHERE 't';
INSERT INTO bcpi VALUES (1, 1);
-- @expect-error: duplicate key value violates unique constraint
-- @profile-lock
-- @test
INSERT INTO bcpi VALUES (1, 2);
