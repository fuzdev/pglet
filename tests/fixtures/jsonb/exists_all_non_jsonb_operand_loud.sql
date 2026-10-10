-- The `?&` member of the same rule: a non-container left operand is `42883`, where the
-- eval arm, reached without the rule, would answer NULL.
-- @test
-- @expect-error: no integer ?& integer operator
-- @expect-message
SELECT 1 ?& 1;
