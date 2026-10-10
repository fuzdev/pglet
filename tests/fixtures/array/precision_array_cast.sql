-- A `[]` makes the precision the **element's**, applied element-wise through the one
-- `apply_typmod` — 18.6's `ArrayCoerceExpr`, which runs the element cast per element.
--
-- `timestamptz(3)[]` is deliberately absent: pglet's array **text** encoder drops the
-- element's `+00` offset, a pre-existing divergence unrelated to the precision (it
-- shows on a bare `timestamptz[]` too) and recorded in `EDGES.md`. Its store path is
-- covered by `ddl/precision_column_store.sql`'s scalar `timestamptz(3)` column.
-- @test
SELECT '{2020-01-02 03:04:05.6789,2020-01-02 03:04:05.4}'::timestamp(3)[] AS a,
       '{1.23456 secs,-0.0015 secs}'::interval(3)[] AS b,
       '{03:04:05.6789}'::time(3)[] AS c;
