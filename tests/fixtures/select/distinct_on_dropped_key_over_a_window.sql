-- @setup
CREATE TABLE dw (q INT, a INT);
INSERT INTO dw VALUES (1, 10), (1, 20), (2, 30);

-- The third node kind that can carry a junk column. `row_number()` is numbered over
-- the input order and the dedup runs above it, so the surviving rows keep the numbers
-- they were given before the collapse — 1 and 3, not 1 and 2.
-- @test
SELECT DISTINCT ON (q) a, row_number() OVER () AS rn FROM dw ORDER BY q;
