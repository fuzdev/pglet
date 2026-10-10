-- The same pair in a **write**'s predicate. `DELETE FROM t WHERE m > s` was `DELETE 1`
-- here, taking the `happy` row 18.6 keeps by refusing the statement. The gate is plan-time,
-- above any row, so nothing is written before it answers — `crates/pglet`'s
-- `enum_identity_gates` test binary is what asserts the survivors, which an
-- `@expect-error` fixture structurally cannot.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood > text
-- @expect-message
DELETE FROM t WHERE m > s;
