-- A foreign key 18.6 cannot implement — referencing type `int4[]`, referenced key
-- `int8[]`. `int4[] → int8[]` *is* an implicit coercion, element-wise, and still refused: the key's
-- class is `array_ops`, whose input type `anyarray` must bind to one array type on both sides.
-- @setup
CREATE TABLE ftp (k INT8[] PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: int4[] and int8[]
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k INT4[] REFERENCES ftp);
