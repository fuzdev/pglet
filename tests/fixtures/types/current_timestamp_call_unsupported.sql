-- `CURRENT_TIMESTAMP` is a parenless value function, not a callable function, so
-- an argument list fails loud — matching Postgres, which also rejects
-- `current_timestamp()`. (A bare `CURRENT_TIMESTAMP` is supported; use `now()`
-- for the callable form.)
-- @test
-- @expect-error: current_timestamp() takes no argument list
-- @no-error-fields: both engines send `P` and point at different tokens — 18.6 at the closing paren, pglet at the opening one
SELECT current_timestamp();
