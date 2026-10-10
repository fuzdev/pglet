-- The spine's `COALESCE(SUM(size), 0)::bigint` over an empty `BIGINT` column.
-- @setup
CREATE TABLE fact (hash TEXT, size BIGINT);
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0)::bigint AS total FROM fact;
