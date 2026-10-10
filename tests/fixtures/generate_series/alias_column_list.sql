-- A column list names the column; the alias still names the relation.
-- @test
SELECT i, g.i FROM generate_series(4, 6) g(i);
