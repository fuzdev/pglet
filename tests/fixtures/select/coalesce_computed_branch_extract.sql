-- `EXTRACT` is `numeric` as a branch: `COALESCE(i + 0, extract(epoch FROM ts))` is
-- `numeric` and keeps the epoch's fraction, and an `avg` over it is `numeric` too.
-- @setup
CREATE TABLE t (i INT, ts TIMESTAMP);
INSERT INTO t VALUES (NULL, '2020-01-02 03:04:05.5'), (4, NULL);
-- @test
SELECT COALESCE(i + 0, extract(epoch FROM ts)) AS e, COALESCE(i, extract(epoch FROM ts)) AS e2,
  (SELECT avg(COALESCE(i + 0, extract(epoch FROM ts))) FROM t) AS a
FROM t ORDER BY i NULLS FIRST;
