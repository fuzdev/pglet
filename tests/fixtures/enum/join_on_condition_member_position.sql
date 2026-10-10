-- An enum comparison inside a **join `ON` condition** binds through the join's own
-- column map (`JoinColumnMap`), which is a different column-resolution object from the
-- scan's `SimpleColumns` and owns its label lists rather than borrowing them. The
-- `WHERE`/`ORDER BY` spellings in `join_order_by_member_position` never reach it — they
-- run at the `Filter` and `Sort` above the join, over its output descriptors.
--
-- A `LEFT JOIN` deliberately, so a non-matching row survives as NULLs: if the condition
-- picked the wrong rows both the matched and the unmatched sides move, which a plain
-- inner join would show only as a shorter result.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (id INT, tag TEXT);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1, 'a'), (2, 'b'), (3, 'c');

-- @test
SELECT t.id, u.tag
FROM t LEFT JOIN u ON u.id = t.id AND t.m > 'sad'
ORDER BY t.id;
