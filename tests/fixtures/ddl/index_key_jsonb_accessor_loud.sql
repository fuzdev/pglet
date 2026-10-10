-- The sibling of `index_predicate_jsonb_accessor_loud` on the **key expression**
-- side: `starts_with(data->'k', 'x')` has no `starts_with(jsonb, text)` signature,
-- and the NULL-propagating probe can't see it either.
-- @setup
CREATE TABLE jk2 (id INT, data JSONB);
-- @test
-- @expect-error: no starts_with(jsonb, text)
CREATE INDEX jk2_i ON jk2 (starts_with(data->'k', 'x'));
