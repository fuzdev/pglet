-- The aggregate node builds its descriptors on the same walk, so a qualified `t.c`
-- in a `GROUP BY` projection or inside `max(t.c)` was declared `text` there too.
-- Aggregate output names are bare (projection drops the qualifier), so this covers
-- the loose resolution tier rather than the alias tier — and the aggregate's own
-- result type, which follows its argument's and so inherited the same wrong answer.
-- @setup
CREATE TABLE qad (id INT PRIMARY KEY, grp INT, flag BOOL, amt FLOAT8);
INSERT INTO qad VALUES (1, 7, true, 1.5), (2, 7, true, 2.5), (3, 8, false, 4.0);

-- @test
SELECT qad.grp, qad.flag, max(qad.amt) AS top, count(*) AS n
FROM qad GROUP BY qad.grp, qad.flag ORDER BY qad.grp;
