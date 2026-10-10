-- A NULL array element sorts GREATER than any non-NULL element (Postgres
-- `array_cmp`), so `{1,NULL}` sorts LAST, not first. Regression guard for the
-- sign-inverted NULL-element arm in `values_compare`.
-- @setup
CREATE TABLE arr (id INT, v INT[]);
INSERT INTO arr VALUES (1, ARRAY[1,2]), (2, ARRAY[1,0]), (3, ARRAY[1,NULL]), (4, ARRAY[1]);
-- @test
SELECT v FROM arr ORDER BY v;
