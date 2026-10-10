-- An unknown relation inside an `ANY ((SELECT …))` sub-select is `42P01` on both engines,
-- ahead of pglet's refusal of the sub-select form.
-- @expect-error: relation "nosuch" does not exist
-- @test
SELECT 1 = ANY ((SELECT 1 FROM nosuch));
