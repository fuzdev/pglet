-- `ALTER TABLE <an index> RENAME TO` is refused with the wrong kind's `42809` here; 18.7
-- renames the index (its `ALTER TABLE` takes any relation), so any pglet error passes.
-- The same holds for a sequence named through `ALTER TABLE`.
-- @profile-lock
-- @setup
CREATE TABLE ri (a int);
CREATE INDEX ri_a ON ri (a);
-- @expect-error: cannot open relation "ri_a"
-- @test
ALTER TABLE ri_a RENAME TO ri_b;
