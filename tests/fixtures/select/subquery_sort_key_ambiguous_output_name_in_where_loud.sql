-- @setup
CREATE TABLE sjw1 (n INT, x INT);
CREATE TABLE sjw2 (n INT, y INT);
INSERT INTO sjw1 VALUES (2, 20), (1, 10);
INSERT INTO sjw2 VALUES (1, 100), (2, 200);

-- The control for `subquery_sort_key_ambiguous_output_name_loud.sql`: the identical
-- ambiguity in `WHERE` over the identical zero-row join. It was `42702` on both engines
-- throughout, which is what identifies that fixture's divergence as the sort-key path
-- rather than the plan shape or the empty join.
-- @expect-error: a bare name two joined relations claim is 42702 in WHERE
-- @test
SELECT sjw1.x FROM sjw1 JOIN sjw2 ON false WHERE n = 1;
