-- `RETURNING` is a projection, and it was the last clause reaching neither the coercion
-- pass nor the gate. Unfolded, `COALESCE(i4, i8)` declared `integer` (both walkers read
-- the first branch) over a value whose common type is `bigint` — 4 bytes declared, 8
-- sent, per row, which a binary client mis-decodes. The declared type is the assertion:
-- the rendered values agree either way, so only the `RowDescription` tells the two apart.
-- The rows are arranged so a different branch wins on each one.
-- @setup
CREATE TABLE urc (id INT, i4 INT4, i8 INT8);
INSERT INTO urc VALUES (1, 7, NULL), (2, NULL, 200);
-- @test
UPDATE urc SET id = id RETURNING id, COALESCE(i4, i8) AS c, GREATEST(i4, i8) AS g, ARRAY[i4, i8] AS a;
