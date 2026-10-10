-- Non-correlated `EXISTS(SELECT ...)`: the inner query references only its own
-- table, so it is a constant predicate from the outer query's perspective —
-- it keeps every outer row when the inner query has a matching row, none
-- otherwise. Also covers a no-FROM projection form `SELECT EXISTS(...) AS e`.
-- @setup
CREATE TABLE t (id INT, name TEXT);
CREATE TABLE u (id INT, flag BOOLEAN);
INSERT INTO t VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO u VALUES (1, true), (2, false);
-- @test
SELECT id FROM t WHERE EXISTS (SELECT 1 FROM u WHERE u.flag = true) ORDER BY id;
