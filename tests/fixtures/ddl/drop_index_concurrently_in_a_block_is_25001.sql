-- The `DROP` half of the `25001` gate — including the case that makes it a *gate*
-- and not a side effect of doing work: `IF EXISTS` naming nothing still raises,
-- because the refusal precedes the lookup on both engines.
-- @setup
CREATE TABLE ti (id INT, v TEXT);
-- @test
-- @expect-error: DROP INDEX CONCURRENTLY cannot run inside a transaction block
DROP INDEX CONCURRENTLY IF EXISTS ti_nope_idx;
