-- The typing occurrence is an operator's operand rather than a cast, in a column list
-- that names the table's last column first and leaves one to its default.
-- @setup
CREATE TABLE up_nk (n INT, k TEXT, v JSONB);
-- @param $1 text z
-- @test
INSERT INTO up_nk (v, k, n) VALUES ('{}', $1 || 'x', CASE WHEN $1 IS NULL THEN 0 ELSE 1 END) RETURNING k, n;
