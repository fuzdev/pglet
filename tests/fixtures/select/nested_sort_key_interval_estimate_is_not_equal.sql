-- @setup
CREATE TABLE sivl1 (c INT, d DATE);
CREATE TABLE sivl2 (k INT);
INSERT INTO sivl1 VALUES (1, '2026-01-31'), (2, '2026-02-28');
INSERT INTO sivl2 VALUES (1), (2);

-- The interval half of `nested_sort_key_numeric_display_scale_is_not_equal.sql`, and the
-- sharper of the two: these two entries do not merely *render* differently, they compute
-- **different dates** — `2026-01-31 + 1 mon` is February, `+ 30 days` is March. 18.6
-- answers `42702` because `equal()` compares the two `Const`s byte-for-byte.
--
-- pglet's `PgInterval` compares by Postgres's own estimate (`micros + (months*30 +
-- days) * USECS_PER_DAY`), so `INTERVAL '30 days' == INTERVAL '1 mon'` — correct and
-- deliberate for SQL value comparison, and wrong as a structural identity. Used as one,
-- it folded these two entries into a single claimant, the `42702` was lost, and the
-- `DELETE` spelling removed every row.
--
-- The subquery is `WHERE false`, so nothing is evaluated: the refusal is the plan-time
-- gate's. The key must be nested — a top-level bare sort key is exempt from that gate.
-- @expect-error: two intervals of equal estimate but different calendar effect are two entries
-- @test
SELECT k FROM sivl2 WHERE EXISTS (SELECT d + INTERVAL '1 mon' AS q, d + INTERVAL '30 days' AS q FROM sivl1 WHERE false ORDER BY q);
