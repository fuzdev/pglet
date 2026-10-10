-- The same order, one node deeper: the inner `ON`'s operand walk reaches its own
-- `42883` before the outer `ON` is resolved at all.
-- @setup
CREATE TABLE bcnj4 (k INT, t TEXT);
CREATE TABLE bcnj5 (k INT, t TEXT);
CREATE TABLE bcnj6 (k INT, t TEXT);
-- @expect-error: operator does not exist: text ~~ integer, not the outer ON's 42804
-- @test
SELECT 1 FROM bcnj4 JOIN bcnj5 ON bcnj4.t LIKE 1 JOIN bcnj6 ON bcnj5.k;
