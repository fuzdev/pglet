-- ...and now that the column-level CHECK carries its name, it collides like any other
-- constraint of that name on the table (42710).
-- @expect-error: constraint dupn for relation ctcc already exists
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ctcc (a INT CONSTRAINT dupn CHECK (a > 0), CONSTRAINT dupn UNIQUE (a));
