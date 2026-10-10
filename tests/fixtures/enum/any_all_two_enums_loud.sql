-- The same door over a **second enum**'s array — a member of `EDGES.md` row 115's
-- closed class, beside the `ARRAY[…]` *pair* spelling over the same two columns, which
-- both engines refuse (`mood[] < m3[]`) and which is what made the two look like one fact.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood > m3
-- @expect-message
SELECT id, (m > ANY(ARRAY[y])) AS a FROM w ORDER BY id;
