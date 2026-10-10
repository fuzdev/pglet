-- `COALESCE(SUM(b), 0)` over an empty `BIGINT` column with no cast: the `0` is the answer,
-- and it is the `numeric` `0` the unification makes it, not an `int4`.
-- @setup
CREATE TABLE fact (hash TEXT, size BIGINT);
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0) AS total FROM fact;
