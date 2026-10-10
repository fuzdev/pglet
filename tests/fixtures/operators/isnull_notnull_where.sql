-- A `WHERE x ISNULL` filters on the operand's nullness, not on its value.
-- @setup
CREATE TABLE inn (id INT, a INT);
INSERT INTO inn VALUES (1, NULL), (2, 5);
-- @test
SELECT id, a ISNULL AS n FROM inn WHERE a NOTNULL OR id = 1 ORDER BY id;
