-- Same refusal for the index behind an inline UNIQUE constraint (2BP01) — the
-- marker records the constraint kind, not just "is a PK".
-- @setup
CREATE TABLE diuq (id INT PRIMARY KEY, k INT UNIQUE);

-- @expect-error: cannot drop index diuq_k_key because constraint diuq_k_key requires it
-- @no-error-fields: pglet's HINT names the only removal it implements; 18.6 suggests ALTER TABLE ... DROP CONSTRAINT, which pglet does not implement (it is refused 0A000)
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX diuq_k_key;
