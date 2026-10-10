-- `setval` is STRICT: a NULL argument skips the call entirely and the result is NULL.
-- pglet raised 42804 on the argument coercion instead.
-- @setup
CREATE SEQUENCE svn1;
-- @test
SELECT setval('svn1', NULL);
