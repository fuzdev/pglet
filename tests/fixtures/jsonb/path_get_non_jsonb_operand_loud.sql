-- `#>` is `jsonb #> text[]`, so `1 #> 1` resolves to no operator on 18.6; without the
-- rule, pglet's path arm would answer a wrong-shape NULL.
-- @test
-- @expect-error: no integer #> integer operator
-- @expect-message
SELECT 1 #> 1;
