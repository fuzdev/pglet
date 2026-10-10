-- The `INSERT … SELECT` door of the short row: the source produces fewer columns than the
-- list names — `42601 INSERT has more target columns than expressions`.
-- @setup
CREATE TABLE isct (a INT, b INT);

-- @test
-- @expect-error: INSERT has more target columns than expressions (42601)
-- @expect-message
INSERT INTO isct (a, b) SELECT 1;
