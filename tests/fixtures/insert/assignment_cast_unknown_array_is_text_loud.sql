-- An `ARRAY[…]` of unknown constants is **not** unknown: `transformArrayExpr` resolves
-- its elements to a common type, which for all-unknown input is `text`, and `text[] →
-- uuid[]` is no more assignable than `text → uuid` is. So the carve-out for Postgres's
-- `unknown` stops at the bare constant and does not follow it into a construct — the
-- same reason `CASE WHEN true THEN 't' ELSE 'f' END` and `COALESCE('5','6')` are
-- `42804` into a non-text column on both engines.
--
-- The cast spellings of this very value are accepted on both
-- (`ARRAY[…]::UUID[]`, `ARRAY['…'::UUID]`), which is what identifies the construct's
-- resolution rather than the value as the refusal.
-- @setup
CREATE TABLE acua (id INT, ua UUID[]);

-- @expect-error: column "ua" is of type uuid[] but expression is of type text[] (42804)
-- @test
INSERT INTO acua (id, ua) VALUES (1, ARRAY['11111111-1111-1111-1111-111111111111']);
