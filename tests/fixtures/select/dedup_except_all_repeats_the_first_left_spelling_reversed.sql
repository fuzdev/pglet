-- @test
-- The reversed input order of `dedup_except_all_repeats_the_first_left_spelling`, over a
-- `numeric[]`: the surplus repeats `{1.0}`.
SELECT x FROM (VALUES (ARRAY[1.0]), (ARRAY[1.00]), (ARRAY[1.000])) a(x)
EXCEPT ALL SELECT ARRAY[1];
