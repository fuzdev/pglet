-- date/timestamp/timestamptz cross-compare in Postgres, so the operand gate must
-- accept every pairing. `time` and `interval` are deliberately *not* members —
-- `date = time` and `timestamp = interval` have no operator (see
-- `compare_temporal_family_reject_loud.sql`).
-- @setup
CREATE TABLE t (d DATE, ts TIMESTAMP, tz TIMESTAMPTZ);
INSERT INTO t VALUES ('2024-06-15', '2024-06-15 00:00:00', '2024-06-15 00:00:00+00');
-- @test
SELECT d = ts AS d_ts, d = tz AS d_tz, ts = tz AS ts_tz,
       d < ts + INTERVAL '1 day' AS lt, tz >= d AS ge
FROM t;
