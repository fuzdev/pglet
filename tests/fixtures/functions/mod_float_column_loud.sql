-- No float `mod` over a column either: 42883.
-- @setup
CREATE TABLE mf (r REAL);
INSERT INTO mf VALUES (1.5);
-- @test
-- @expect-error: function mod(real, integer) does not exist
SELECT mod(r, 2) FROM mf;
