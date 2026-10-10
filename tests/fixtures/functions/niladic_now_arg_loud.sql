-- `now` resolves by name *and* argument types on Postgres, so `now(integer)` names a
-- function that does not exist (42883). pglet's `build_call` used to route the niladic
-- impure builtins by name alone and discard the parsed argument list, answering an
-- ordinary timestamp here — a silent wrong in every expression position, since
-- `build_call` is the one site a function call is built at.
-- @test
-- @expect-error: function now(integer) does not exist
SELECT now(1);
