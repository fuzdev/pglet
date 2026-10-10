-- TRUNCATE reads the same relkind rule as DROP TABLE (`"s" is not a table`).
-- @setup
CREATE SEQUENCE trs_seq;
-- @test
TRUNCATE trs_seq;
-- @expect-error: a sequence is not a table (42809)
