-- @test
-- @expect-error: a DEFAULT string is read at DDL, 22P02
CREATE TABLE bytea_input_dl (b BYTEA DEFAULT '\X61');
