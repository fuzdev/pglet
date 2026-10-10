-- A foreign key 18.6 cannot implement — referencing type `float8`, referenced key
-- `numeric`. The implicit coercion runs one way: `numeric → float8` is implicit (so `numeric` referencing
-- `float8` is accepted — `type_implicit_coercion_to_the_key_accepted`) and `float8 → numeric` is not.
-- @setup
CREATE TABLE ftp (k NUMERIC PRIMARY KEY);

-- @expect-error: 42804 foreign key constraint cannot be implemented: float8 and numeric
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
CREATE TABLE ftc (k FLOAT8 REFERENCES ftp);
