-- The same INTERNAL-edge shape, and the one place pglet deliberately sends a hint
-- 18.6 does not: 18.6 suggests `ALTER TABLE … DROP CONSTRAINT`, which pglet does not
-- parse, so pointing a reader at it would be worse than diverging. Message locked,
-- hint waived.
-- @setup
CREATE TABLE t_ci (a INT PRIMARY KEY);

-- @test
-- @expect-error: the constraint requires its index
-- @no-profile-lock: the 2BP01 comes from the same catalog-only dependency resolver — the index here is the *object being dropped*, never one selection could choose, and `DROP INDEX` plans no predicate
-- @expect-message
-- @no-error-fields: pglet's HINT names the only removal it implements; 18.6 suggests ALTER TABLE ... DROP CONSTRAINT, which pglet does not implement (it is refused 0A000)
DROP INDEX t_ci_pkey;
