-- A searched `CASE WHEN` over a non-boolean silently took the `ELSE` arm.
-- @expect-error: argument of CASE/WHEN must be type boolean
-- @test
SELECT CASE WHEN 1 THEN 'y' ELSE 'n' END;
