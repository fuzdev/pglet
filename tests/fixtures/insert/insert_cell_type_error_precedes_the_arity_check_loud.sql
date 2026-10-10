-- A row's cells are transformed before its length is compared with the target list, so an
-- ill-typed cell in a row that is also too long is the cell's `42804`, not the arity's
-- `42601`.
-- @setup
CREATE TABLE icta (a INT, b INT);

-- @test
-- @expect-error: COALESCE types integer and text cannot be matched (42804), ahead of the arity's 42601
INSERT INTO icta (a, b) VALUES (COALESCE(1, 'x'::TEXT), 2, 3);
