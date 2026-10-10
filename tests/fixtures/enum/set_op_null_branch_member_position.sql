-- A `NULL` branch is the other shape that declares no type — same rule, and the one a
-- reader is likeliest to assume is special. Its row sorts last under the default
-- `NULLS LAST`, which is the same on both engines and is not what this pins.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, x::text AS lbl
FROM (SELECT id, m AS x FROM t UNION ALL SELECT 0, NULL) q
ORDER BY x, id;
