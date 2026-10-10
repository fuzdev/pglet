-- An unknown operator over an aggregate names the aggregate's type in its refusal:
-- `count(*)` is `bigint` whatever its argument. The message is built while the statement
-- is parsed, before the coercion pass has decided any aggregate's argument.
-- @setup
CREATE TABLE ou (a INT, b INT);
INSERT INTO ou VALUES (1, 10), (2, 0);
-- @test
-- @expect-error: operator does not exist: bigint ! integer
-- @expect-message
SELECT count(*) ! 1 FROM ou;
