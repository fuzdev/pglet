-- The reversed pair: a lone surrogate ahead of \u0000 is the `22P02` the surrogate
-- earns, not `22P05`.
-- @test
-- @expect-error: the lone surrogate is first
SELECT '["\ud800", "\u0000"]'::jsonb;
