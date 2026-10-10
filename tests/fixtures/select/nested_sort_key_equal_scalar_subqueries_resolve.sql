-- @setup
CREATE TABLE ssq1 (c INT);
CREATE TABLE ssq2 (k INT);
INSERT INTO ssq2 VALUES (1), (2);

-- The plan-bearing half of the `equal()` rule the rest of this family covers over
-- scalar entries: two target entries that are each a **scalar subquery**, written
-- identically. `findTargetlistEntrySQL92` compares them with `equal()`, which descends
-- the whole `SubLink` including its `Query`, finds them identical, and counts one
-- claimant — so 18.6 answers rather than raising, and `_loud`'s twin (the same shape
-- with `(SELECT 2)`) is `42702`.
--
-- pglet's claimant identity is `planner::coercion`'s `structural_identity`, which
-- encodes the entry through `planner::expr`'s **ungated** `encode_expr`. That is why
-- the encoder is split from the persistence-bound `serialize_expr`, which refuses a
-- plan-bearing `Expr`: routing this comparison through the gate makes both entries
-- identity-less, each its own claimant, and this statement a spurious `42702` —
-- measured on either side of the split, which is the whole reason this fixture exists.
-- @test
SELECT k FROM ssq2 WHERE NOT EXISTS (SELECT (SELECT 1) AS q, (SELECT 1) AS q FROM ssq1 WHERE false ORDER BY q) ORDER BY k;
