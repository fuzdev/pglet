-- `SUM(bigint)` is `numeric`, so a total past `bigint` is computed; the `::bigint` that
-- casts it back is what refuses it, `22003`.
-- @setup
CREATE TABLE fact (hash TEXT, size BIGINT);
INSERT INTO fact VALUES ('a', 9223372036854775807), ('b', 1);
-- @test
-- @expect-error: bigint out of range
SELECT COALESCE(SUM(size), 0)::bigint AS total FROM fact;
