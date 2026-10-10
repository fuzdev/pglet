-- The sequence arm's own precedence, measured rather than reasoned. `'bad name'` is not
-- a well-formed relation name, and pglet reduces the literal inside `sequence_call` —
-- but 18.6 resolves that argument's `regclass` cast *after* `ParseFuncOrColumn`, so the
-- quantifier wins there. The verdict therefore sits between this arm's arity check and
-- its name reduction. 42809 on both; pglet answered 42602 before.
-- @test
-- @expect-error: DISTINCT specified, but nextval is not an aggregate function
SELECT nextval(DISTINCT 'bad name');
