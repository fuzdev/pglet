-- `transformFromClauseItem` transforms `j->larg` and `j->rarg` before the join's own
-- quals, so an enclosed join's `ON` is decided before the join enclosing it: the
-- inner `ON`'s unbindable column answers, not the outer `ON`'s non-boolean operand.
-- @setup
CREATE TABLE bcnj1 (k INT, t TEXT);
CREATE TABLE bcnj2 (k INT, t TEXT);
CREATE TABLE bcnj3 (k INT, t TEXT);
-- @expect-error: column bcnj1.nosuch does not exist, not the outer ON's 42804
-- @test
SELECT 1 FROM bcnj1 JOIN bcnj2 ON bcnj1.nosuch JOIN bcnj3 ON bcnj2.k;
