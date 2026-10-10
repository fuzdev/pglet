-- The mirror spelling, which answered the same wrong row.
-- @setup
CREATE TABLE bca2 (a INT);
INSERT INTO bca2 VALUES (9);
-- @expect-error: argument of AND must be type boolean
-- @test
SELECT count(*) FROM bca2 WHERE true AND a;
