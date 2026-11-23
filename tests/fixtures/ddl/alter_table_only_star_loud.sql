-- `ONLY t*` is not `relation_expr`: `ONLY` takes a bare name, so the `*` is a syntax
-- error, before any relation is looked up.
-- @expect-error: syntax error at the star
-- @no-error-fields: both engines send a position, and pglet's parser may word the error differently
-- @test
ALTER TABLE ONLY nosuch_atos* ADD COLUMN d INT;
