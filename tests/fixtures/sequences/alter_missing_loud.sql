-- An ALTER SEQUENCE naming nothing is 42P01. The statement used to be a total no-op,
-- so this succeeded silently.
-- @test
ALTER SEQUENCE no_such_sequence_here RESTART;
-- @expect-error: relation does not exist (42P01)
