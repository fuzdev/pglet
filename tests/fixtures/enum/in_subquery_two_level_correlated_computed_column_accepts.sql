-- The same leaf two levels down, where the enclosing chain is what carries `w1`: the
-- innermost subquery's own input is `w3`, and `w1.m` is two namespaces out.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
SELECT id,
       (m IN (SELECT (SELECT COALESCE(w1.m, 'ok') FROM w w3 LIMIT 1) FROM w w2 LIMIT 1)) AS a
FROM w w1 ORDER BY id;
