-- The type-unify cast is applied POSITIONALLY, not by column name. Both columns of
-- the left side are unnamed (`?column?`), and the left casts int4 -> int8 to match the
-- right. A by-name cast would resolve every `?column?` to the first column, replacing
-- the second column's 5 with the first's 1 (silent corruption); positional casting
-- keeps 5. Expected: (1, 5), (2, 6).
-- @test
SELECT 1, 5
UNION ALL
SELECT 2::int8, 6::int8
ORDER BY 1;
