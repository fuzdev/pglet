-- `OWNED BY <table>.<column>` resolves against the live catalog: a column the table
-- lacks is 42703. pglet used to parse the target and throw it away.
-- @setup
CREATE TABLE own_host (a INT);
-- @test
CREATE SEQUENCE own_bad_col OWNED BY own_host.nosuch;
-- @expect-error: column does not exist (42703)
