-- The `SELECT DISTINCT` member, and the one that was **never silent**: a bogus key there
-- was refused `42P10 for SELECT DISTINCT, ORDER BY expressions must appear in select list`
-- where 18.6 says `42703 column "bogus" does not exist`. A wrong code rather than a wrong
-- answer, and row-count-**independent** — hence the seeded relation, where the other
-- members of this class need an empty one to reproduce.
--
-- The cause is the same blanket: `SELECT DISTINCT`'s rule is that every sort key appear in
-- the select list, so a key binding to nothing failed *that* test first. With the key bound
-- against the target list before the DISTINCT rule runs, the name error comes first, as it
-- does in `transformSelectStmt` (sort is transformed before distinct).
-- @setup
CREATE TABLE sk_dist (a INT, v INT);
INSERT INTO sk_dist VALUES (1, 10), (2, 20);

-- @expect-error: column "bogus" does not exist (42703)
-- @test
SELECT DISTINCT a FROM sk_dist ORDER BY bogus;
