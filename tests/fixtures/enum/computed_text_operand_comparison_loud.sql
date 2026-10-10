-- The read polarity of the same pair, with a function result standing where a `text`
-- column would. Its siblings take the other operand shapes one apiece; nothing here
-- claims the set is complete.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: no operator for mood > text
-- @expect-message
SELECT m > lower(s) FROM t;
