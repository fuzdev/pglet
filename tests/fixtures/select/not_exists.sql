-- `NOT EXISTS(SELECT ...)`: the negation of a non-correlated existence test. The
-- inner query has no matching row, so `NOT EXISTS` is true and every outer row
-- passes the filter. (Swap the inner predicate to a matching value and the
-- filter would drop every row.)
-- @setup
CREATE TABLE t (id INT, name TEXT);
CREATE TABLE u (id INT, flag BOOLEAN);
INSERT INTO t VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO u VALUES (1, true), (2, false);
-- @test
SELECT id FROM t WHERE NOT EXISTS (SELECT 1 FROM u WHERE u.flag = false AND u.id = 99) ORDER BY id;
