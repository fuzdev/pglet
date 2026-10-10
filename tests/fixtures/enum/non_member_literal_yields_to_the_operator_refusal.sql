-- Which enum error wins, at one scale down from the statement: 18.6 resolves the operator
-- and only then coerces the `unknown` side, so an operator refusal in an earlier `AND`
-- argument takes the statement and the literal never reaches its conversion.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK');

-- @test
-- @expect-error: the operator refusal in the first argument outranks the literal in the second
-- @expect-message
SELECT m > lower(s) AND m > 'zzz' FROM t;
