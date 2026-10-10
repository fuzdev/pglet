-- `NULLIF` reports a **different code** for the same operand pair, and that is the
-- point: it resolves an `=` operator rather than a common type, so `time` against
-- `timestamp` is `42883` ("operator does not exist") where `greatest`/`COALESCE`/`UNION`
-- over the identical pair are `42846`. pglet used to declare `text` (OID 25) here and
-- then send eight bytes of `time` microseconds under it.
-- @setup
CREATE TABLE ntt (tm TIME, ts TIMESTAMP);
INSERT INTO ntt VALUES ('10:00:00', '2026-01-01 10:00:00');
-- @test
-- @expect-error: no `time = timestamp` operator (42883)
SELECT nullif(tm, ts) FROM ntt;
