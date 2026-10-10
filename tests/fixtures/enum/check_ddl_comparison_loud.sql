-- The DDL-time half of the same rulebook. A `CHECK` over the pair was **persisted** here
-- and refused there, so the accepted catalog dumped to SQL stock 18.6 rejects on restore.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood <> text
-- @expect-message
CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (m <> s));
