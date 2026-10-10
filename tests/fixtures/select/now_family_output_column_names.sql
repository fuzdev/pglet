-- Each `now`-family spelling names its own output column, which is what a derived
-- table's auto-named column exposes: on 18.6 `SELECT clock_timestamp()` projects a
-- column called `clock_timestamp`, so `s.clock_timestamp` resolves through the derived
-- table (measured for all five spellings).
--
-- pglet named all five `now`, so four of these five references were `42703` — the same
-- collapse the `column_default` rendering showed, reaching a plain `SELECT`.
--
-- The values are clock reads, so each column is reduced to a boolean; what is being
-- asserted is that the *name* resolves at all.
-- @test
SELECT (SELECT s.now FROM (SELECT now()) s) IS NOT NULL AS n1,
       (SELECT s.transaction_timestamp FROM (SELECT transaction_timestamp()) s) IS NOT NULL AS n2,
       (SELECT s.statement_timestamp FROM (SELECT statement_timestamp()) s) IS NOT NULL AS n3,
       (SELECT s.clock_timestamp FROM (SELECT clock_timestamp()) s) IS NOT NULL AS n4,
       (SELECT s.current_timestamp FROM (SELECT CURRENT_TIMESTAMP) s) IS NOT NULL AS n5;
