-- An explicit `x::varchar(n)` cast **always** truncates an over-length value
-- silently, even when the excess is non-blank (Postgres explicit-cast semantics,
-- unlike the column store which errors on non-blank excess). `'abcd'::varchar(3)` →
-- `'abc'`. Truncation is character-based (UTF-8), not byte-based.
-- @test
SELECT 'abcd'::varchar(3) AS v, 'héllo'::varchar(3) AS m;
