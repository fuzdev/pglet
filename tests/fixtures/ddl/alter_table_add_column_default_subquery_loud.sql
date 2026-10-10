-- The `ALTER TABLE … ADD COLUMN … DEFAULT` route to a persisted DEFAULT — the third
-- statement that can put one in the catalog, and the one that also rewrites rows.
-- It pins 18.6's SQLSTATE on this route — `0A000 cannot use subquery in DEFAULT
-- expression` — and the absence of a phantom column is what the *next* statement in
-- a session would show, not something this fixture's error comparison can see.
-- @expect-error: cannot use subquery in DEFAULT expression (0A000)
-- @setup
CREATE TABLE acdsq (a INT);
-- @test
ALTER TABLE acdsq ADD COLUMN b INT DEFAULT (SELECT 1);
