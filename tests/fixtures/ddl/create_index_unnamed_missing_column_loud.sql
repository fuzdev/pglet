-- A key column the table lacks is `42703`, as with a name — the name is chosen from the
-- key, and a key that does not resolve never gets one.
-- @setup
CREATE TABLE ciu23 (a INT);
-- @test
CREATE INDEX ON ciu23 (nosuchcol);
-- @expect-error: column "nosuchcol" does not exist (42703)
