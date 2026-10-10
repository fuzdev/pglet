-- `name = integer` and `name + integer` resolve no operator on 18.6. pglet refuses
-- both too; the arithmetic one takes its own older `42804` spelling where 18.6 says
-- `42883`, which is the pre-existing wrong-SQLSTATE-class row every non-numeric
-- operand hits (`'a'::text + 1` measures the identical split), so the comparison is
-- the one pinned here.
-- @expect-error: no name = integer operator
-- @test
SELECT current_user = 1;
