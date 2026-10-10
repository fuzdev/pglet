-- A `[]` does not make a modifier legal: `_int4`'s `typmodin` mirrors `int4`'s (0),
-- so `int4(3)[]` is the same `42601` — and the message names the array,
-- `"int4[]"`, because `TypeNameToString` prints one `[]` for the suffix.
-- @expect-error: an array of a type that carries no modifier
-- @expect-message
-- @test
SELECT 1::int4(3)[] AS never_reached;
