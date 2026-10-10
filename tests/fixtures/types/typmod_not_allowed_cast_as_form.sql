-- The `CAST(… AS …)` spelling reads the same type-name grammar as `::`, so it takes
-- the same refusal — `bool` carries no modifier either.
-- @expect-error: bool takes no type modifier
-- @expect-message
-- @test
SELECT CAST(true AS bool(3)) AS never_reached;
