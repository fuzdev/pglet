-- The seeding idiom: `INSERT … SELECT … FROM generate_series(…)`.
-- @setup
CREATE TABLE gs_seed (id INT PRIMARY KEY, label TEXT, n BIGINT);
INSERT INTO gs_seed SELECT g, 'row ' || g, g * 10 FROM generate_series(1, 500) AS g;
INSERT INTO gs_seed (id) SELECT * FROM generate_series(501, 502);
-- @test
SELECT count(*), sum(id), min(label), max(label), sum(n), count(n) FROM gs_seed;
