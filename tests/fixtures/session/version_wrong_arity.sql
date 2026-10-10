-- The arity check is the whole reason a name may not be implemented without one:
-- without it this spelling would be *accepted* and answer the banner, which is a
-- silent wrong rather than the `42883` both engines owe.
-- @expect-error: no version(integer) signature
-- @test
SELECT version(1);
