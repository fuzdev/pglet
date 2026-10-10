-- The gate's **reach**, at a write. `lower(s)` is a computed operand, which the
-- plan-time operand reader answers `None` for — so the pair carried the enum identity
-- and the gate was handed nothing to judge, and the `DELETE` took the row 18.6 keeps by
-- refusing. The surviving rows are the assertion the `@expect-error` cannot make.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: no operator for mood > text, whatever computes the text side
-- @expect-message
DELETE FROM t WHERE m > lower(s);
