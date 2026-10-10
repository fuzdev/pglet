-- A fold that lands on an enum target does not leave the `unknown` literal alone: 18.6
-- runs `enum_in` on it, and a label the type does not hold is `22P02` in parse analysis.
-- pglet holds an enum value as a `Value::Text`, so skipping the coercion answered the
-- label back.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT greatest(m, 'zzz') FROM t;
