-- Two columns of the **same** enum are one type, so every enum-identity gate has to
-- let them through — the identity rule is equality, not presence.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, n mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
SELECT id, (m > n) AS gt, greatest(m, n)::text AS g, COALESCE(m, n)::text AS c,
       nullif(m, n)::text AS nz
  FROM t ORDER BY id;
