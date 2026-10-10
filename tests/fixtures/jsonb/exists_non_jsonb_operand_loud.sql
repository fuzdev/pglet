-- `?` is registered only as `jsonb ? text`, so an integer pair resolves to no operator
-- and 18.6 is `42883`. The shared rule gates the pair at plan time, at DDL time and at
-- eval alike; without it the eval arm would answer the wrong-shape operand with a silent
-- NULL.
-- @test
-- @expect-error: no integer ? integer operator
-- @expect-message
SELECT 1 ? 1;
