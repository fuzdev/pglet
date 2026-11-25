-- A fold's `$N` branch beside a `jsonb` one is `jsonb`.
-- @setup
CREATE TABLE jfp (id INT, j JSONB);
INSERT INTO jfp VALUES (1, '{"k": [1]}'), (2, '{"x": 1}');
-- @param $1 jsonb []
-- @test
SELECT id, coalesce(j -> 'k', $1) || '[2]' AS v FROM jfp ORDER BY id;
