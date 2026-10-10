-- The `VALUES` half of the alternative: a bare `INSERT INTO t DEFAULT` is a syntax
-- error on 18.6 (`insert_rest` has no lone-`DEFAULT` production), and the arm's
-- `expect_kw(Keyword::Values)` is what refuses it here. The target carries a live
-- default, which is what makes the fixture discriminate — an engine that consumed
-- `DEFAULT` and treated the missing keyword as optional would **succeed**, inserting
-- an all-defaults row, rather than answering some other error.

-- @expect-error: DEFAULT without VALUES is a syntax error (42601)
-- @setup
CREATE TABLE dv_kw (id INT, v TEXT DEFAULT 'x');

-- @test
INSERT INTO dv_kw DEFAULT;
