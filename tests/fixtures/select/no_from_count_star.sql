-- `SELECT count(*)` with no `FROM`: a grouping query over the one empty row, which counts `1`.
-- @test
SELECT count(*);
