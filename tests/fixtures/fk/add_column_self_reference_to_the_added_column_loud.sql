-- An `ADD COLUMN … REFERENCES` naming the added column itself as the referenced key finds it — the key
-- is added in the pass after the column's — so the answer is the covering key's `42830`, not a missing
-- column's `42703`.
-- @setup
CREATE TABLE fsa (z INT);

-- @expect-error: 42830 no unique constraint matching given keys
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE fsa ADD COLUMN a TEXT REFERENCES fsa (a);
