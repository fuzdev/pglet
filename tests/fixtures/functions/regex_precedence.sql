-- `~` sits in the "any other operator" band with `||`, left-associative, so
-- `'ab' || 'c' ~ '^abc'` is `('ab' || 'c') ~ '^abc'`; a comparison binds looser, so
-- `= true` compares the match.
-- @test
SELECT 'ab' || 'c' ~ '^abc' AS concat_first, 'ab' ~ '^a' = true AS compared, 'x' ~ '^x' IS TRUE AS is_true;
