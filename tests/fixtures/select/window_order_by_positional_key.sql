-- @setup
CREATE TABLE wz (a INT, b INT);
INSERT INTO wz VALUES (1, 5), (1, 7), (2, 5);

-- The other output-answered key kind: a positional ordinal names the Nth output
-- column and needs no junk entry. Made junk instead it would be a constant, which
-- sorts nothing and leaves the input order — so `DESC` over the row numbers is what
-- makes the difference visible.
-- @test
SELECT b AS a, row_number() OVER () AS rn FROM wz ORDER BY 2 DESC;
