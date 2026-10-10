-- A set-returning `unnest` item in a `WHERE` `IN` list is `0A000` on both engines, under a
-- NULL left operand as under any other.
-- @setup
CREATE TABLE tn (x INT);
INSERT INTO tn VALUES (NULL);
-- @expect-error: set-returning functions are not allowed in WHERE
-- @test
SELECT count(*) FROM tn WHERE x IN (1, unnest(ARRAY[1, 2]));
