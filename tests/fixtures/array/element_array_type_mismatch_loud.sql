-- The same correction on the other side of the split: two array-typed elements share a
-- type category, so the pair is Postgres's `42846` "could not convert", not `42804` and
-- not a nesting complaint. Both codes come out of the one `select_common_type` fold now
-- that the nesting verdict is taken after it rather than before.
-- @setup
CREATE TABLE aat (ia INT4[], ta TEXT[]);
INSERT INTO aat VALUES (ARRAY[1,2], ARRAY['a']);
-- @expect-error: ARRAY element types share a category but do not convert (42846)
-- @test
SELECT ARRAY[ia, ta] FROM aat;
