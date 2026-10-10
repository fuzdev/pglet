-- `DISTINCT ON` keeps the first row of each group *in the effective sort order*, so the
-- member order picks a **different row per group** — not merely a different row order.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, g INT);
INSERT INTO t VALUES (1, 'sad', 1), (2, 'ok', 1), (3, 'happy', 2), (4, 'ok', 2);

-- @test
SELECT g, id, m::text AS label
FROM (SELECT DISTINCT ON (g) g, id, m FROM t ORDER BY g, m) s
ORDER BY g;
