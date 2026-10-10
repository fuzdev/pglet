-- The placement, as a cross-code pair: the predicate's `42804` beats the key
-- column's `42703`, because 18.6 analyses the whole `IndexStmt` before
-- `DefineIndex` resolves a bare key column.
-- @setup
CREATE TABLE bcix2 (a INT);
-- @expect-error: argument of WHERE must be type boolean, not the key column's 42703
-- @no-profile-lock: a DDL-time refusal; no index is built and no plan is chosen.
-- @test
CREATE INDEX bcix2_ix ON bcix2 (nosuchcol) WHERE a;
