-- Two `bigint`s past 2^53 that differ by one are different values to every comparison:
-- `ORDER BY`, `min`, `count(DISTINCT …)`, a `>` filter, `BETWEEN`, `least`, a window's
-- peers and a `<` join. The column carries no index, so the answers are the comparator's
-- own.
-- @setup
CREATE TABLE cki_8 (x INT8);
INSERT INTO cki_8 VALUES (9007199254740992), (9007199254740993), (9007199254740994), (-9007199254740993), (-9007199254740992);
-- @test
SELECT 'order' AS q, x FROM (SELECT x FROM cki_8 ORDER BY x DESC LIMIT 2) s
UNION ALL SELECT 'min', min(x) FROM cki_8
UNION ALL SELECT 'distinct', count(DISTINCT x) FROM cki_8
UNION ALL SELECT 'above', x FROM cki_8 WHERE x > 9007199254740992
UNION ALL SELECT 'between', count(*) FROM cki_8 WHERE x BETWEEN 9007199254740993 AND 9007199254740993
UNION ALL SELECT 'least', least(-9007199254740992::int8, -9007199254740993::int8)
UNION ALL SELECT 'rank', rank() OVER (ORDER BY x) FROM cki_8
UNION ALL SELECT 'below pairs', count(*) FROM cki_8 a JOIN cki_8 b ON a.x < b.x
ORDER BY q, x;
