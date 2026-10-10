-- `COALESCE(avg(<CASE of integers>), 7)` over no rows: the `CASE` is `integer`, so the
-- `avg` is `numeric` and the default is unified to a `numeric` `7` — not to the
-- `double precision` an untyped `avg` argument was guessed as, whose eight bytes a
-- `numeric` reader cannot read.
-- @setup
CREATE TABLE t (i INT);
-- @test
SELECT COALESCE(avg(CASE WHEN i > 0 THEN i ELSE 0 END), 7) AS x FROM t;
