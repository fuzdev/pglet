-- `||` stringifies its operands, and `timestamptz` renders with its `+00` UTC
-- offset while a plain `timestamp` does not — a distinction the `Value` alone
-- cannot carry (both are `Value::Timestamp`), so the concat path needs the
-- operand's *declared* type. Dropping the offset silently shifts the instant for
-- any driver that reparses the string in its own zone. The `::text` cast has the
-- same hazard, so it rides along here.
-- @setup
CREATE TABLE ctz (a TIMESTAMPTZ, b TIMESTAMP);
INSERT INTO ctz VALUES ('2026-07-18 12:30:45+00', '2026-07-18 12:30:45');
-- @test
SELECT
  'v=' || a   AS tz_concat,
  'w=' || b   AS plain_concat,
  a::text     AS tz_cast,
  b::text     AS plain_cast
FROM ctz;
