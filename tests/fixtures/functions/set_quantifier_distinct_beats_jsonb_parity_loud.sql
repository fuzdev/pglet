-- `jsonb_build_object`'s signature is `VARIADIC "any"`, so **any** argument count
-- resolves it and the odd-list complaint is an execution-time 22023 on 18.6 — later than
-- the quantifier's check, not earlier. So the verdict is judged inside this arm, ahead
-- of the parity check, where the tail position would have answered pglet's 42804.
-- 42809 on both.
-- @test
-- @expect-error: DISTINCT specified, but jsonb_build_object is not an aggregate function
SELECT jsonb_build_object(DISTINCT 'a');
