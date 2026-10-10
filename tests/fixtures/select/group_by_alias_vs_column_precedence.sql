-- **The inverted-precedence rule.** When a bare `GROUP BY` key could name either a
-- FROM-clause column or a same-named output alias, Postgres binds the **FROM column** —
-- `findTargetlistEntrySQL92`'s `colNameToVar` precheck, taken for `GROUP BY` only:
-- "In GROUP BY, we must prefer a match against a FROM-clause column to one against
-- the targetlist." `ORDER BY` and `DISTINCT ON` have no such precheck, so an alias
-- wins there — the precedence between the two clause families is **opposite**
-- (`group_by_order_by_alias_precedence_contrast.sql` locks the other direction).
--
-- Here `a` is both a column of `pgp` and the alias of the projected `b`. Grouping by
-- the column `pgp.a` (the primary key) makes every column functionally dependent, so
-- the query is valid and yields one row per `a` — three rows. Had the alias won, the
-- grouping would be by `b` and the answer would be two rows. Both spellings run, so a
-- wrong precedence is a **silently wrong row set**, not an error.
-- @setup
CREATE TABLE pgp (a INT PRIMARY KEY, b INT);
INSERT INTO pgp VALUES (1, 10), (2, 10), (3, 20);
-- @test
SELECT b AS a, count(*) AS c FROM pgp GROUP BY a ORDER BY 1, 2;
