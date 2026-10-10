-- The control that keeps the `CASE/WHEN` gate off the **simple** form: 18.6
-- expands `CASE x WHEN v` to `x = v` and coerces the comparison's result, so the
-- label is judged by the `=` operator rule and an integer label is ordinary SQL.
-- @test
SELECT CASE 1 WHEN 1 THEN 'y' ELSE 'n' END;
