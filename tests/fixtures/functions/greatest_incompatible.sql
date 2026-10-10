-- greatest with an unknown string literal next to an integer: Postgres resolves the
-- common type to integer and coerces 'a'::int, which fails to parse (22P02) — not a
-- silent pick. pglet reproduces this via the same unknown-literal coercion.
-- @test
-- @expect-error: greatest('a', 1) coerces 'a' to int and fails to parse
SELECT greatest('a', 1);
