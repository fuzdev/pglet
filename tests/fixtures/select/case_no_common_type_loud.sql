-- The same `42804` as COALESCE, through `CASE`'s branch resolution.
-- @setup
CREATE TABLE cs (a INT, t TEXT);
INSERT INTO cs VALUES (1, 'x');
-- @test
-- @expect-error: no common type for integer and text (42804)
SELECT CASE WHEN a = 1 THEN a ELSE t END FROM cs;
