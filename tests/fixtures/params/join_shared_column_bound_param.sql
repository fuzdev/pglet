-- A bound `$N` over a scope whose relations share a column name. The parameter has no
-- type until bind and the plan is cached across binds, so the static comparison gate
-- must fall through on it rather than judge the pair — while the *other* conjunct,
-- whose operands are both concrete, is still resolved qualifier-aware.
--
-- That second conjunct is the load-bearing half, and it must name the relation the
-- bare-name merge got **wrong**: `jbp_b` is registered last, so a flat map keyed on the
-- bare `x` typed every `x` as its `text` and `jbp_a.x = '10'` was left comparing
-- `integer` against an uncoerced `text` literal. Written against `jbp_b.x` instead, the
-- merge answers correctly by accident and the fixture asserts nothing.
-- @setup
CREATE TABLE jbp_a (id INT, x INT);
CREATE TABLE jbp_b (id INT, x TEXT);
INSERT INTO jbp_a VALUES (1, 10), (2, 20);
INSERT INTO jbp_b VALUES (1, 'k'), (2, 'm');
-- @param $1 int4 10
-- @test
SELECT jbp_a.id, jbp_a.x FROM jbp_a JOIN jbp_b ON jbp_a.id = jbp_b.id WHERE jbp_a.x = $1 AND jbp_a.x = '10';
