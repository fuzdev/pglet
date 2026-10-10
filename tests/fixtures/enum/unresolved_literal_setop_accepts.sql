-- An `unknown`-typed term on either side of a set operation adopts the enum, in both
-- orders — the accept side of the door the `text`-column term is refused at.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, n mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
SELECT x::text AS x FROM (SELECT 'sad' AS x UNION ALL SELECT m FROM t) u ORDER BY x;
