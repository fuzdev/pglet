-- A qualifier that names no FROM item is 42P01, the series' alias notwithstanding.
-- @expect-error: missing FROM-clause entry for table zzz
-- @test
SELECT zzz.g FROM generate_series(1, 2) g;
