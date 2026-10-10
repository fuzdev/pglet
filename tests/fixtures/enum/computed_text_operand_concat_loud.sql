-- A concatenation is the computed operand here, and it is the same verdict: what the gate
-- reads is the operand's type, not which node produced it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: no operator for mood > text
-- @expect-message
SELECT m > (s || '') FROM t;
