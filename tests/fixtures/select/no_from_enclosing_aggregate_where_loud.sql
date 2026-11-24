-- The no-FROM `WHERE` spelling of an aggregate over the enclosing query's column: refused
-- `0A000` here; 18.7 answers one row, so any pglet error passes.
-- @setup
CREATE TABLE r11 (a int);
INSERT INTO r11 VALUES (1), (2), (3);
-- @expect-error: an aggregate over an enclosing query's column in a SELECT with no FROM
-- @test
SELECT (SELECT max(r11.a) WHERE true) FROM r11;
