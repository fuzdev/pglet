-- `OR` names itself, and a projection position reaches the gate as a clause
-- position does.
-- @expect-error: argument of OR must be type boolean
-- @test
SELECT 1 OR false;
