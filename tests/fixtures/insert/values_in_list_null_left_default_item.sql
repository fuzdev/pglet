-- `DEFAULT` as an `IN` list item inside an `INSERT … VALUES` row is `42601` on both
-- engines, under a NULL left operand as under any other.
-- @setup
CREATE TABLE u (id INT, v INT);
-- @expect-error: DEFAULT is not allowed in this context
-- @test
INSERT INTO u VALUES (1, (NULL::int IN (1, DEFAULT))::int);
