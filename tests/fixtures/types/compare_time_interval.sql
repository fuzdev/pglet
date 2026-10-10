-- Postgres implicitly casts `time → interval` (`pg_cast`, castcontext `i`), so
-- `time` and `interval` compare in either operand order — the one cross-family
-- pair the gate accepts. The comparison is the interval estimate, so
-- `'24:00:00'::time = INTERVAL '1 day'` (a day estimates as 24 h).
-- @setup
CREATE TABLE t (tm TIME, iv INTERVAL);
INSERT INTO t VALUES ('01:00:00', '1 hour');
-- @test
SELECT tm = iv AS eq, tm = INTERVAL '2 hours' AS neq,
       tm < INTERVAL '2 hours' AS lt, iv > '00:30:00'::time AS gt,
       '24:00:00'::time = INTERVAL '1 day' AS day_endpoint,
       iv = tm AS reversed
FROM t;
