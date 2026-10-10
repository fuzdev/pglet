-- Positional walk, aggregate/subquery pair, polarity 2 of 2 — the mirror of
-- `default_aggregate_before_subquery.sql`. The subquery is reached first, so 18.6
-- answers `0A000 cannot use subquery in DEFAULT expression` where the mirror
-- answers `42803`.
-- @expect-error: subquery before the aggregate wins (0A000)
-- @test
CREATE TABLE dff (a INT, b INT DEFAULT ((SELECT 1) + count(*)));
