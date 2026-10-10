-- An enum column beside a plain `text` column has no comparison operator on 18.6 — every
-- comparison operator an enum has is `anyenum`'s — and the OID alone cannot see it, an
-- enum's being `text`. pglet answered by the **member positions** and kept the `happy` row
-- (`EDGES.md` row 114); the refusal is `42883` at plan time, so it holds over an empty
-- table too.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood > text
-- @expect-message
SELECT id, m::text AS label FROM t WHERE m > s ORDER BY id;
