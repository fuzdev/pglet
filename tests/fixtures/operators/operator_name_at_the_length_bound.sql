-- The control beside the refusal: 63 characters is one below `NAMEDATALEN`, so the
-- name reaches analysis and both engines answer the same sentence for it.
-- @test
-- @expect-error: operator does not exist at 63 characters (42883)
-- @expect-message
SELECT 1 *************************************************************** 1;
