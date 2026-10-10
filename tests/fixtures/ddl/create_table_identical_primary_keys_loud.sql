-- Two PRIMARY KEYs over the *same* column is still 42P16, not a merge. Postgres runs
-- the multiplicity check per constraint in `transformIndexConstraint`, before the
-- redundant-specification de-duplication its caller does — so identical UNIQUEs merge
-- and identical PRIMARY KEYs do not.
-- @expect-error: multiple primary keys for table ctip are not allowed
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctip (a INT NOT NULL, PRIMARY KEY (a), PRIMARY KEY (a));
