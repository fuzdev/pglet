-- An `IN` element is Postgres's `x = elem`, so it takes the operator refusal and not the
-- fold's — with the element computed, as with a column.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT m IN (lower(s)) FROM t;
