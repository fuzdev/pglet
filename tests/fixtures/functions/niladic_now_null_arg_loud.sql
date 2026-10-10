-- A NULL argument is an argument: 18.6 resolves the call as `now(unknown)` and finds no
-- such function (42883), and pglet's gate counts the list rather than inspecting it, so
-- the two agree. The spelling is worth pinning on its own because a gate written to
-- reject "a value" rather than "a list" would let it through — `Literal::Null` is the
-- one argument shape several walkers elsewhere in the engine treat as absent.
-- @test
-- @expect-error: function now(unknown) does not exist
SELECT now(NULL);
