-- Two *different* enum columns. 18.6 has no operator for the pair, and neither has
-- pglet: `42883`. Comparing by the **label bytes** — neither column's member positions —
-- is the answer this refuses (`EDGES.md` row 115, closed).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE v (id INT, m mood NOT NULL, y m3 NOT NULL);
INSERT INTO v VALUES (1, 'ok', 'sad'), (2, 'ok', 'happy'), (3, 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood > m3
-- @expect-message
SELECT id, (m > y) FROM v ORDER BY id;
