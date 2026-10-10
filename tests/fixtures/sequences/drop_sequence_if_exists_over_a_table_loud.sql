-- The wrong kind beats the `IF EXISTS` skip, and this is the silent member of the
-- class: "does nothing" is a legal outcome here, so a skip would report success and
-- the next statement to name `dsiet_t` would insist it already exists.
-- @setup
CREATE TABLE dsiet_t (a INT);
-- @test
DROP SEQUENCE IF EXISTS dsiet_t;
-- @expect-error: a table is not a sequence, `IF EXISTS` notwithstanding (42809)
