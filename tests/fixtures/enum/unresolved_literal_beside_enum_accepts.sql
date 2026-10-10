-- **The over-correction polarity.** A bare string literal is 18.6's `unknown`: it adopts
-- the position's type, enum included, so every one of these resolves to `mood` there and
-- must not meet the refusal the `text`-column pair earns. Each answer is the **member**
-- order — `'ok'` is the middle member and the byte order is `happy < ok < sad` — so a
-- fixture that merely stopped refusing would still be wrong here.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, n mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
SELECT id,
       (m = 'ok') AS eq,
       (m > 'ok') AS gt,
       COALESCE(m, 'sad')::text AS coal,
       greatest(m, 'ok')::text AS g,
       least(m, 'ok')::text AS l,
       nullif(m, 'ok')::text AS nz
  FROM t ORDER BY id;
