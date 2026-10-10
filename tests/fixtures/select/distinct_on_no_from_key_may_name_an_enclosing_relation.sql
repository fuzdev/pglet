-- The correlated accept polarity of the no-FROM `DISTINCT ON` binding gate: inside an
-- expression subquery the level's scope is incomplete, so the gate must stay OPEN and
-- leave the key to the correlated re-walk — an unconditional close refuses this
-- ordinary statement (`42P01` naming `ct`) where both engines answer a row. No sibling
-- fixture can see that regression: every refusing fixture expects an error either way,
-- and the alias-key control names an output column, which a closed scope still answers.
-- The outer `AS r` sidesteps a separate scalar-subquery output-naming divergence
-- (pglet says `?column?` where 18.6 propagates the inner alias `q`).
-- @setup
CREATE TABLE ct (a INT);
INSERT INTO ct VALUES (7);
-- @test
SELECT (SELECT DISTINCT ON (ct.a) 1 AS q) AS r FROM ct;
