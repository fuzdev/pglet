-- `ANY ((SELECT …))` is the sub-select form, which pglet refuses, but the refusal stands in
-- for no 18.6 error and so comes after every error 18.6 raises in the statement: an
-- unknown column in the left operand is `42703` on both engines.
-- @expect-error: column "nosuch" does not exist
-- @test
SELECT nosuch = ANY ((SELECT 1));
