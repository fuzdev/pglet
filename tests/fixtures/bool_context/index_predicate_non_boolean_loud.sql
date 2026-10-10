-- A partial index's predicate is the fifth clause 18.6 spells `"WHERE"`, and its
-- gate outranks the index's own key columns and its access method.
-- @setup
CREATE TABLE bcix (a INT, b TEXT);
-- @expect-error: argument of WHERE must be type boolean
-- @no-profile-lock: the refusal is at DDL time in the `CREATE INDEX` executor,
-- which runs identically on both profiles; no index is ever built to select.
-- @test
CREATE INDEX bcix_ix ON bcix (b) WHERE a;
