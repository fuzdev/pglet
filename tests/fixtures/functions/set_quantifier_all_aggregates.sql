-- `ALL` on the aggregates, where it is the explicit spelling of the default: over
-- `1, 1, 2` each answer is the whole-column one, not the deduplicated one, so this pins
-- that carrying the keyword does not accidentally set the aggregate's `distinct` flag.
-- Its `DISTINCT` twin (`set_quantifier_distinct_aggregates.sql`) is the other polarity.
-- @setup
CREATE TABLE sq_all_agg (a INT);
INSERT INTO sq_all_agg VALUES (1), (1), (2);
-- @test
SELECT count(ALL a) AS c, sum(ALL a) AS s, min(ALL a) AS mn, max(ALL a) AS mx
FROM sq_all_agg;
