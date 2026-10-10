-- MIN/MAX over an array column with a NULL element: MAX is `{1,NULL}` (a NULL
-- element sorts greatest), MIN is `{1,0}`. Exercises the ordering-backed aggregate
-- path, not just the bare operators.
-- @setup
CREATE TABLE arr (v INT[]);
INSERT INTO arr VALUES (ARRAY[1,2]), (ARRAY[1,0]), (ARRAY[1,NULL]);
-- @test
SELECT min(v) AS lo, max(v) AS hi FROM arr;
