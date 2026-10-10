-- A table carries a row type of its own name, so `heap_create_with_catalog` tests the
-- type namespace for the new table's name right behind the relation namespace: over an
-- enum of that name it is `42710`, with the hint 18.6 sends on this route only.
-- Before the type namespace was modelled pglet built both, and the dump then restored
-- short (the `CREATE TABLE` refused, the table and its rows lost, `psql` exiting `0`).
-- @setup
CREATE TYPE tnt1 AS ENUM ('x');
-- @expect-error: a table over an enum's name
-- @expect-message
-- @test
CREATE TABLE tnt1 (a INT);
