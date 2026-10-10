-- The `UPDATE` polarity of the silent no-FROM bound (see
-- `select/no_from_limit_zero_in_a_delete.sql` for the class): `OFFSET 1` over a one-row
-- no-FROM select is `(0 rows)` on 18.6, so the `IN` matches nothing and no row changes —
-- while the discarded bound left the row visible here and overwrote one with `99`.
-- @setup
CREATE TABLE ofw (a INT);
INSERT INTO ofw VALUES (1), (2), (3);
UPDATE ofw SET a = 99 WHERE a IN (SELECT 1 AS q OFFSET 1);

-- @test
SELECT a FROM ofw ORDER BY a;
