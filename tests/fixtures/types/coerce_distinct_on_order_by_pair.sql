-- Pair symmetry: the `DISTINCT ON` keys and the `ORDER BY` keys are compared by
-- structural `==` (`merge_distinct_on_order_by`, which requires the ON exprs to head
-- the ORDER BY). Both are resolved by the same pass against the same `ColumnScope`,
-- so `(x = '1')` becomes the identical `x = '1'::int4` in each and still matches.
-- Coercing one alone would raise a spurious `42P10` — or, if the mismatch fell the
-- other way, silently dedup on a different grouping.
--
-- `x` is projected bare on purpose: pglet has no resjunk columns, so a `DISTINCT ON`
-- key over a column the projection drops fails `42703` for an unrelated reason (see
-- `select/distinct_on_dropped_key_loud.sql`).
-- @setup
CREATE TABLE gt (x INT, tag TEXT);
INSERT INTO gt VALUES (1, 'a'), (1, 'b'), (2, 'c');
-- @test
SELECT DISTINCT ON (x = '1') x FROM gt ORDER BY (x = '1'), x;
