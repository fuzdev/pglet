-- A sequence is an object of the schema on its own: over a `public` holding one and
-- nothing else, `DROP SCHEMA public` without `CASCADE` is 18.6's `2BP01`
-- (`sequence dsq_s depends on schema public`), not a drop that clears the sequence and its
-- counter. The setup empties `public` first, inside the isolating wrapper, so a table
-- another fixture left behind cannot answer the refusal in the sequence's place.
-- @expect-error: a schema holding only a sequence needs CASCADE
-- @setup
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
CREATE SEQUENCE dsq_s;
-- @test
DROP SCHEMA public;
