-- The prefix form over an aggregate, in `HAVING`.
-- @setup
CREATE TABLE ou (a INT, b INT);
INSERT INTO ou VALUES (1, 10), (2, 0);
-- @test
-- @expect-error: operator does not exist
SELECT a FROM ou GROUP BY a HAVING (!! avg(b)) > 0;
