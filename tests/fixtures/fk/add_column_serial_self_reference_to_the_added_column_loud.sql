-- The rewrite branch of a self-reference to the added column — a `SERIAL` over rows: the referenced
-- list resolves in the post-ADD columns there too, so the covering key's `42830` answers.
-- @setup
CREATE TABLE fsc (z INT);
INSERT INTO fsc VALUES (1);

-- @expect-error: 42830 no unique constraint matching given keys
-- @expect-message
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
ALTER TABLE fsc ADD COLUMN a SERIAL REFERENCES fsc (a);
