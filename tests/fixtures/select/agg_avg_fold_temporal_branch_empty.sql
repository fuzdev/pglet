-- The same fold over no rows: `avg` is `double precision` on both legs, and the
-- `COALESCE` default beside it is cast to that, not to `numeric`.
-- @setup
CREATE TABLE t (f FLOAT8, d DATE, d2 DATE);
INSERT INTO t VALUES (1.5, '2020-01-02', '2020-01-01');
-- @test
SELECT COALESCE(avg(COALESCE(d - d2, f + 0)), 7) FROM t WHERE false;
