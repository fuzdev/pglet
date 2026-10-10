-- A `GROUP BY` **ordinal** under a JOIN. The ordinal resolves to the target-list
-- entry's *expression*, so the grouping is by `gpja.x` — two groups.
--
-- pglet used to leave the key as the literal `1`, collapsing every joined row into a
-- single bucket: a silently wrong row set, not an error. Nothing caught it because the
-- plan-time grouping-validity check returns early for a multi-table query, so the bare
-- constant key that `42803`s on a single table sails through here.
-- @setup
CREATE TABLE gpja (id INT, x INT);
CREATE TABLE gpjb (id INT, y INT);
INSERT INTO gpja VALUES (1, 10), (2, 10), (3, 20);
INSERT INTO gpjb VALUES (1, 100), (2, 200), (3, 300);
-- @test
SELECT gpja.x, count(*) AS c FROM gpja JOIN gpjb ON gpja.id = gpjb.id GROUP BY 1 ORDER BY 1;
