-- `COALESCE(SUM(b), 7)` over an empty `BIGINT` column: the default is unified to the
-- `numeric` `SUM(bigint)` is, so it is sent as a `numeric` 7 — an `int8` 7's eight bytes
-- read as a `numeric` are a zero of scale 7, where a `0` default's would read as a zero.
-- @setup
CREATE TABLE fact (hash TEXT, size BIGINT);
-- @test
SELECT COALESCE(SUM(size), 7) AS total, COALESCE(SUM(size), 7)::bigint AS cast_total FROM fact;
