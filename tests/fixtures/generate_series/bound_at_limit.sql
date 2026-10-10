-- A series of exactly the row bound answers.
-- @test
SELECT count(*), sum(g), min(g), max(g) FROM generate_series(1, 1000000) g;
