-- Each row of a multi-row `VALUES` is analysed in its written order too.
-- @setup
CREATE TABLE up_nk (n INT, k TEXT, v JSONB);
-- @param $1 text z
-- @test
INSERT INTO up_nk (k, n) VALUES ($1::text, CASE WHEN $1 IS NULL THEN 0 ELSE 1 END), ('q', 2) RETURNING k, n;
