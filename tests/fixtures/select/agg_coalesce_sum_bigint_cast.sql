-- The spine's `COALESCE(SUM(size), 0)::bigint` over a filled `BIGINT` column: the `numeric`
-- total is cast back to `bigint`.
-- @setup
CREATE TABLE fact (hash TEXT, size BIGINT);
INSERT INTO fact VALUES ('a', 10), ('b', 4611686018427387904), ('c', NULL), ('d', 4611686018427387000);
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0)::bigint AS total FROM fact;
