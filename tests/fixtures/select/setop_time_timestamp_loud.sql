-- `time` and `timestamp` are both `typcategory` D, but **neither is implicitly
-- coercible to the other**, so a set-op column mixing them has no common type: `42846`
-- ("UNION could not convert type timestamp without time zone to time without time
-- zone"), not the `42804` a cross-category pair gets.
-- This is the shape that proved the common-type fold must carry its own invariant. The
-- fold used to keep the running type whenever the candidate didn't displace it, without
-- checking the candidate could convert *to* it — and because `timestamp → time` is a
-- supported truncating cast, the bogus target silently succeeded and returned
-- `10:00:00` from a timestamp instead of erroring.
-- @setup
CREATE TABLE stt (tm TIME, ts TIMESTAMP);
INSERT INTO stt VALUES ('10:00:00', '2026-01-01 10:00:00');
-- @test
-- @expect-error: time and timestamp share a category but do not convert (42846)
SELECT tm AS x FROM stt UNION ALL SELECT ts FROM stt;
