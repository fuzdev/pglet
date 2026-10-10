-- The same pair written the other way round is also 42710: the CHECK is attached as
-- the heap is created, so it holds the name by the time the constraint index is
-- built, whichever order they appear in the statement.
-- @expect-error: constraint n for relation ctpc already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctpc (a INT NOT NULL, CONSTRAINT n PRIMARY KEY (a), CONSTRAINT n CHECK (a > 0));
