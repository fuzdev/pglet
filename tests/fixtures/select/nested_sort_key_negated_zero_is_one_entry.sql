-- @setup
CREATE TABLE snz1 (c INT, x INT);
CREATE TABLE snz2 (k INT);
INSERT INTO snz1 VALUES (1, 10), (2, 20);
INSERT INTO snz2 VALUES (1), (2);

-- The false rejection the unary-minus fold closes, and it is a **value** fixture because
-- 18.6 answers both spellings. `findTargetlistEntrySQL92` compares two same-named target
-- entries with `equal()`, which on a `Const` is `datumIsEqual` — and there `0` and `-0`
-- are one constant, because `doNegate` folded the minus into the constant before parse
-- analysis ever built a node to compare. pglet kept an `Expr::Negate` wrapper, so the two
-- entries were structurally distinct and the key was `42702` on a statement both engines
-- run. Recorded as one of the causes behind that gate's over-rejection; the fold retires
-- it, and `nested_sort_key_negated_literal_is_a_distinct_entry_loud.sql` is the control
-- that keeps the identity a value equality rather than a blanket collapse.
--
-- Both numeric families are here because they fold through different code: the integer
-- pair negates an `i64` (`0` has no sign) and the decimal pair negates a `PgNumeric`
-- (whose zero is likewise unsigned — `SELECT -0.0` renders `0.0` on both engines, and
-- `0.0 = -0.0` is true).
--
-- Every subquery is `WHERE false`, so nothing is evaluated and the refusal, when there
-- was one, was the plan-time gate's. The key must be **nested**: a top-level bare sort
-- key is exempt from that gate entirely.
-- @test
SELECT k,
       EXISTS (SELECT 0 AS q, -0 AS q FROM snz1 WHERE false ORDER BY q) AS int_zero,
       EXISTS (SELECT 0.0 AS q, -0.0 AS q FROM snz1 WHERE false ORDER BY q) AS numeric_zero
FROM snz2
ORDER BY k;
