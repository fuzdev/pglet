-- A FROM-clause `generate_series` with no alias: the relation and its column are both
-- named `generate_series`, and the column is `integer`.
-- @test
SELECT * FROM generate_series(1, 3);
