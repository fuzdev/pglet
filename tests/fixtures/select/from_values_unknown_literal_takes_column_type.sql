-- A `VALUES` column is resolved with `select_common_type` over all its cells, where a
-- bare string constant contributes no type: `'2'` takes `1`'s, and a leading `NULL`
-- and `'2'` take `3`'s.
-- @test
SELECT * FROM (VALUES (1, NULL), ('2', '2'), (3, 3)) v(a, b) ORDER BY 1;
