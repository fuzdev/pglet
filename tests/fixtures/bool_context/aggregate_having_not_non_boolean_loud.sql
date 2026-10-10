-- `NOT` over a non-boolean reached through a grouping query's `HAVING` is 18.6's `42804`,
-- as anywhere else.
-- @setup
CREATE TABLE bcag4 (b TEXT);
INSERT INTO bcag4 VALUES ('x'), ('y');
-- @expect-error: argument of NOT must be type boolean
-- @test
SELECT b FROM bcag4 GROUP BY b HAVING NOT 1;
