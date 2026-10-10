-- A `$1` whose only occurrence is an `IS NULL` operand. A null test coerces nothing, so
-- 18.6's parse analysis leaves the parameter without a type and refuses the statement
-- `42P18` at `Parse`, whatever the rows. Answering it means running against whatever
-- type the client's value happens to carry.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
-- @expect-error: 18.6 leaves the parameter untyped: 42P18
-- @expect-message
SELECT name FROM up_t WHERE $1 IS NULL;
