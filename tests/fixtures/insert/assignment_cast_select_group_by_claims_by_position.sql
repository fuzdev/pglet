-- **A false-rejection control — do not trim it.** `addTargetToGroupList` coerces the
-- entry a `GROUP BY` key **names** and no other, so `GROUP BY 1` leaves the second
-- entry's `unknown` for the `BOOL` column to resolve. Accepted on 18.6, where the
-- `GROUP BY 1, 2` spelling is `42804`.
--
-- The two entries are the same expression, so a claim compared by *expression* marks
-- both and this becomes a spurious `42804`; its mirror
-- `assignment_cast_select_group_by_ordinal_claims_the_second_entry.sql` is the other
-- half, which a first-match rule gets wrong instead. Only the position gets both.
-- @setup
CREATE TABLE acgp (t TEXT, b BOOL);
CREATE TABLE acgpsrc (n INT);
INSERT INTO acgpsrc VALUES (1), (2);
-- @test
INSERT INTO acgp (t, b) SELECT 't', 't' FROM acgpsrc GROUP BY 1 RETURNING t, b;
