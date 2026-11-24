-- An aggregate in a no-FROM subquery whose argument names the enclosing query's column
-- belongs to the enclosing level on 18.7, which answers one row; pglet has no
-- outer-level aggregates (EDGES row 303) and refuses it `0A000` rather than answer per
-- outer row, so any pglet error passes.
-- @setup
CREATE TABLE r11 (a int);
INSERT INTO r11 VALUES (1), (2), (3);
-- @expect-error: an aggregate over an enclosing query's column in a SELECT with no FROM
-- @test
SELECT (SELECT max(r11.a)) FROM r11;
