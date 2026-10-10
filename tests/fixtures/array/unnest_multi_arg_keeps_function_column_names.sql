-- The **`nfuncs == 1`** half of `chooseScalarFunctionAlias`, from the other side.
--
-- Postgres expands a multi-argument `unnest` in `FROM` into one function per argument,
-- so the alias names the composite row and every column keeps the function name
-- `unnest`. Two columns then answer to the same bare name, which is `42702` — where a
-- rule that dropped the single-function condition would name them both `v` and answer
-- `42703` instead. Both engines say `42702`, so the SQLSTATE is compared and the
-- distinction is pinned rather than merely described.
-- @setup

-- @test
-- @expect-error: 42702 — a two-argument unnest keeps `unnest` for both columns, so the bare name is ambiguous
SELECT unnest FROM unnest(ARRAY['a', 'b'], ARRAY['c', 'd']) AS v;
