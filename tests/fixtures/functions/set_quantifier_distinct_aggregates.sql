-- The control the whole quantifier rule has to keep green: the five aggregates take `DISTINCT`
-- legitimately, so over `1, 1, 2` the counts and sums are the deduplicated ones. A gate
-- that refused the quantifier by name rather than by what the call resolved to would
-- redden here.
-- @setup
CREATE TABLE sq_dist_agg (a INT);
INSERT INTO sq_dist_agg VALUES (1), (1), (2);
-- @test
SELECT count(DISTINCT a) AS c, sum(DISTINCT a) AS s, min(DISTINCT a) AS mn,
       max(DISTINCT a) AS mx
FROM sq_dist_agg;
