-- The multi-argument spelling of `niladic_now_arg_loud.sql`. Arity is not the only
-- thing wrong with it and the count is not what pglet reports, but 18.6's SQLSTATE for
-- both is the same one — no `now/2` signature exists either.
-- @test
-- @expect-error: function now(unknown, integer) does not exist
SELECT now('zzz', 42);
