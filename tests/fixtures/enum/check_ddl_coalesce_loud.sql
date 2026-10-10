-- The DDL-time common-type gate. This `CHECK` was accepted and persisted here, and its
-- dump then failed to restore into stock 18.6 while `psql` exited `0` — the catalog half of
-- the dump invariant.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for text and mood
CREATE TABLE ck (m mood NOT NULL, s TEXT NOT NULL, CHECK (COALESCE(s, m) IS NOT NULL));
