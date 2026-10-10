-- An explicit `x::numeric(p,s)` cast enforces its type modifier (context-insensitive
-- — same as the column store): the value rounds to `scale`, so `12.345::numeric(4,1)`
-- → `12.3`. Previously the cast-level typmod was dropped and the value returned
-- unchanged; the `CastTypmod` IR now applies it.
-- @test
SELECT 12.345::numeric(4,1) AS n;
