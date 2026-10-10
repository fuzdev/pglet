-- A self-reference to the added column, over a table that has a key elsewhere and rows, with the
-- `ALTER` target schema-qualified and the referenced table not: the referenced list still resolves in
-- the post-ADD columns, and the added column has no covering key (`42830`).
-- @setup
CREATE TABLE fsb (z INT UNIQUE);
INSERT INTO fsb VALUES (1), (2);

-- @expect-error: 42830 no unique constraint matching given keys
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE public.fsb ADD COLUMN a BIGINT REFERENCES fsb (a);
