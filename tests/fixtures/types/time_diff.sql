-- `time - time → interval`, here compared against a known interval (a boolean
-- result). Covers a positive span, the reversed (negative) span, and a same-time
-- zero. (Projecting a `time - time` interval is covered by `interval_ts_diff`.)
-- @test
SELECT '13:00:00'::time - '10:00:00'::time = INTERVAL '3 hours'  AS pos,
       '10:00:00'::time - '13:00:00'::time = INTERVAL '-3 hours' AS neg,
       '12:00:00'::time - '12:00:00'::time = INTERVAL '0 hours'  AS zero;
