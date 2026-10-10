-- Columns the row leaves to their defaults, and a written `DEFAULT`, take no place in the
-- order: the operator's operand is still written ahead of the null test.
-- @setup
CREATE TABLE up_def (a INT DEFAULT 7, b TEXT DEFAULT 'd', c TEXT, d INT, e JSONB);
-- @param $1 text z
-- @test
INSERT INTO up_def (d, c, a) VALUES (DEFAULT, $1 || 'x', CASE WHEN $1 IS NULL THEN 0 ELSE 1 END) RETURNING a, b, c, d;
