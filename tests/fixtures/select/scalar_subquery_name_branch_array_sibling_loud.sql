-- The category the string rows do **not** reach: `name` is category `S` and `text[]` is
-- `A`, so the pair has no common type at all and the code is the cross-category one.
-- Which of the two no-common-type codes comes out is what says `name` sits in the string
-- category rather than in the `_` arm's `Array`, where the pair shared a category and
-- earned `42846` instead.
-- @setup
CREATE TABLE ssna (ta TEXT[]);
INSERT INTO ssna VALUES (ARRAY['zzz']);
-- @test
SELECT COALESCE((SELECT current_schema), ta) FROM ssna;
-- @expect-error: COALESCE types name and text[] cannot be matched
