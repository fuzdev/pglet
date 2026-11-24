-- A fourth dotted part is a hard error, not a soft NULL: `makeRangeVarFromNameList`'s
-- `42601`.
-- @expect-error: improper relation name
-- @test
SELECT to_regclass('a.b.c.d');
