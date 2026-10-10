-- An `ARRAY[…]` over an enum column and an unknown literal is a `mood[]` on 18.6, so the
-- array arm of the rulebook must take the element's identity without refusing the literal
-- beside it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, n mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
SELECT id, ARRAY[m, 'ok']::text[] AS a FROM t ORDER BY id;
