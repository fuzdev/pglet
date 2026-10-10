-- A column that is unknown on both sides is `text`: `01` and `1` stay two rows under
-- `UNION`, where resolving them to `integer` would collapse them to one.
-- @test
SELECT '01' AS v UNION SELECT '1' ORDER BY 1;
