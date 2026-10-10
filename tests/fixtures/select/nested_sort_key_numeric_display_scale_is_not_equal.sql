-- @setup
CREATE TABLE snum1 (c INT, x INT);
CREATE TABLE snum2 (k INT);
INSERT INTO snum1 VALUES (1, 10), (2, 20);
INSERT INTO snum2 VALUES (1), (2);

-- `findTargetlistEntrySQL92`'s ambiguity test is Postgres's `equal()`, and on a `Const`
-- that bottoms out in `datumIsEqual` — a `memcmp` of the datum for a by-reference type.
-- It is **structural**, so `1.5` and `1.50` are two different entries and 18.6 answers
-- `42702` here; the pair is the discriminator against pglet's own *value* equality,
-- under which `PgNumeric` folds them (`1.5 == 1.50`, deliberately, for SQL comparison)
-- and the two entries collapsed into one claimant. That answered the `EXISTS` instead of
-- raising, and the `DELETE` spelling of the same statement removed every row.
--
-- `nested_sort_key_duplicate_output_entries_resolve.sql` holds the other polarity, and
-- the same-spelling pair (`1.5 AS q, 1.5 AS q`) resolves on both engines — measured — so
-- what this pins is the display scale and not the literal.
--
-- The subquery is `WHERE false`, so nothing is evaluated: the refusal is the plan-time
-- gate's. The key must be nested — a top-level bare sort key is exempt from that gate.
-- @expect-error: two numeric literals differing only in display scale are two entries
-- @test
SELECT k FROM snum2 WHERE EXISTS (SELECT 1.5 AS q, 1.50 AS q FROM snum1 WHERE false ORDER BY q);
