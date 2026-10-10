-- A sub-plan's target list naming an **enclosing** level's column, where the sub-plan's
-- own FROM is one one-column base table that lacks the name — qualified, bare, and over
-- an empty table. 18.6 binds each to the enclosing column and answers; a binding check
-- that sees only the sub-plan's level refused them `42703`. The `::text` keeps the
-- declared type off the sub-plan's own descriptor (`EDGES.md` row 159).
-- @setup
CREATE TABLE ctl_p (a INT, b INT);
CREATE TABLE ctl_q (c INT);
CREATE TABLE ctl_e (c INT);
INSERT INTO ctl_p VALUES (1, 10), (2, 20);
INSERT INTO ctl_q VALUES (5);
-- @test
SELECT a,
       (SELECT ctl_p.b FROM ctl_q)::text AS qualified,
       (SELECT b FROM ctl_q)::text AS bare,
       (SELECT ctl_p.b FROM ctl_q x)::text AS aliased_inner,
       (SELECT ctl_p.b FROM ctl_e)::text AS empty_inner,
       (SELECT ctl_p.b + 1 FROM ctl_q) AS computed
  FROM ctl_p ORDER BY a;
