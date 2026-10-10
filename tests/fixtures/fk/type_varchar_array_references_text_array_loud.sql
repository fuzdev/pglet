-- A foreign key 18.6 cannot implement — referencing type `varchar[]`, referenced key
-- `text[]`. As `int4[]`/`int8[]`: `varchar → text` is implicit for the scalars and accepted there, but
-- `anyarray` cannot bind `varchar[]` and `text[]` at once.
-- @setup
CREATE TABLE ftp (k TEXT[] PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: varchar[] and text[]
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k VARCHAR[] REFERENCES ftp);
