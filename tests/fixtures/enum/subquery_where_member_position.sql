-- The `WHERE` position, with the subquery as the *right* operand — the sibling arm, and
-- the one `operands_enum_order` could already answer from the enum **column** beside it.
-- Kept as the control that the two operands agree: a subquery leaf answering some other
-- enum's order would change this row set and not the sibling-answered one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT id FROM t WHERE m > (SELECT m FROM u WHERE id = 1) ORDER BY id;
