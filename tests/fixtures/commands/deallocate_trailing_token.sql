-- A second name is a syntax error at *that* token: `DeallocateStmt` takes one target
-- and the statement ends there.
-- @test
-- @expect-error: syntax error at or near "b"
DEALLOCATE a b;
