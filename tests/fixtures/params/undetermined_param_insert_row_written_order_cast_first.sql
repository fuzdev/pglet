-- An `INSERT` row's cells are analysed in the order the row is written. The column list
-- is not in the table's order: `k` is written first and stored second, so the cast is
-- the first occurrence of `$1` although the table puts the null test's column ahead.
-- @setup
CREATE TABLE up_nk (n INT, k TEXT, v JSONB);
-- @param $1 text z
-- @test
INSERT INTO up_nk (k, n) VALUES ($1::text, CASE WHEN $1 IS NULL THEN 0 ELSE 1 END) RETURNING k, n;
