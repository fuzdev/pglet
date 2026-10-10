-- And the kind still beats the skip on this path: `DROP INDEX IF EXISTS <a sequence>`
-- errors rather than reporting success.
-- @setup
CREATE SEQUENCE dise_seq;
-- @test
DROP INDEX IF EXISTS dise_seq;
-- @expect-error: a sequence is not an index, `IF EXISTS` notwithstanding (42809)
