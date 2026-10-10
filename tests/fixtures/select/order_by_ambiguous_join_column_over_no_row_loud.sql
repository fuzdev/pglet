-- The **zero-row twin** of `select/ambiguous_order_by_no_alias.sql`, and the pair is the
-- point: that fixture's join matches rows, so the eval-time resolver raised the `42702`
-- and the plan-time exemption was invisible. Give the join an `ON false` and no row
-- reaches the resolver — this answered `(0 rows)` where 18.6 is `42702` either way.
--
-- The claimant set is what makes it loud: `n` is carried by both sides of the join and by
-- no target-list entry, so `findTargetlistEntrySQL92` falls through to the range table and
-- finds two claimants. Counting them is the rule; declining to count was the blanket.
-- @setup
CREATE TABLE sk_j1 (n INT, x INT);
CREATE TABLE sk_j2 (n INT, y INT);
INSERT INTO sk_j1 VALUES (1, 5);
INSERT INTO sk_j2 VALUES (1, 6);

-- @expect-error: column reference "n" is ambiguous (42702)
-- @test
SELECT sk_j1.x FROM sk_j1 JOIN sk_j2 ON false ORDER BY n;
