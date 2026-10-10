-- `COALESCE(SUM(b), 0)` over a `BIGINT` with no cast: `SUM(bigint)` is `numeric`, so the
-- `0` unifies to `numeric` and the column is declared `numeric` (1700) on both legs.
-- @setup
CREATE TABLE fact (hash TEXT, size BIGINT);
INSERT INTO fact VALUES ('a', 10), ('b', 9223372036854775807), ('c', NULL), ('d', 20);
-- @test
SELECT COUNT(*)::bigint AS count, COALESCE(SUM(size), 0) AS total FROM fact;
