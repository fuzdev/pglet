-- The correlated accept polarity of the no-FROM `ORDER BY` binding gate, and the only
-- fixture that can see an **unconditional** close: inside an expression subquery this
-- level's scope is not whole, so the gate must stay open and leave the key to the
-- enclosing level's `resolve_correlated_subplans` (which re-judges it through
-- `walk_level`'s `Plan::Sort` arm). Closing regardless refuses this ordinary statement
-- `42P01` naming `ct`, where both engines answer a row.
--
-- No refusing sibling can carry that regression — each expects an error either way — and
-- the outer `AS r` sidesteps a separate divergence in the same neighbourhood: pglet names
-- a scalar subquery's output column `?column?` where 18.6 propagates the inner alias.
-- @setup
CREATE TABLE obct (a INT);
INSERT INTO obct VALUES (7);
-- @test
SELECT (SELECT 1 AS q ORDER BY obct.a) AS r FROM obct;
