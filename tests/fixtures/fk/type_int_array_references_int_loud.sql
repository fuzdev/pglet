-- A foreign key 18.6 cannot implement — referencing type `int[]`, referenced key
-- `int`. An array column referencing its element type's key: no family holds the pair and no cast is implicit.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: int[] and int
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k INT[] REFERENCES ftp);
