-- The identity leaf the projection is walked through reaches the **enclosing** levels,
-- and in the order the type leaf reaches them: the entries below name a column of the
-- outer query, which the subquery's own input cannot resolve. A leaf reading the local
-- input alone answers *no enum* for them and refuses every row here.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
SELECT id,
       (m IN (SELECT COALESCE(w1.m, 'ok') FROM w w2)) AS outer_coalesce,
       (m IN (SELECT greatest(w1.m, 'ok') FROM w w2)) AS outer_greatest,
       (m IN (SELECT COALESCE(w2.m, w1.m) FROM w w2)) AS both_levels
FROM w w1 ORDER BY id;
