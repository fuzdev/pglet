-- An `ADD COLUMN … DEFAULT clock_timestamp()` backfill re-reads the clock **per row**;
-- the other four `now`-family spellings take one value for the whole statement.
--
-- 18.6 decides this by the function's volatility: `clock_timestamp()` is VOLATILE and
-- forces a rewrite that calls it once per row, while `now()` /
-- `transaction_timestamp()` are transaction-stable and `statement_timestamp()` /
-- `CURRENT_TIMESTAMP` statement-stable — and one `ALTER` is one statement inside one
-- transaction, so each of those four backfills a single instant. pglet stored all five
-- as one variant and backfilled one value for all of them.
--
-- **The clock column is asserted as `> 1`, not as a count.** 18.6 need not produce
-- one distinct instant per row: consecutive clock reads can share a microsecond, and on
-- one idle host 18.6 gave 3 or 4 over 5 rows every time and 6-11 over 20 (mostly 7),
-- so any exact count is engine-, build- and machine-dependent and a `= 20` assertion
-- was red on Postgres on that host. `> 1` is what both engines satisfy with a margin
-- (measured on that host over 20 rows: 18.6 6-11 distinct, the debug server this
-- harness runs 20, a release server 4-6). The deterministic pin
-- on the classifier itself is `executor::ddl::backfill_volatility_tests`.
--
-- The four `= 1` columns are the discriminating half: a fix that made the whole family
-- volatile would pass the first assertion and fail these.
-- @setup
CREATE TABLE actb (id INT);
INSERT INTO actb SELECT g FROM unnest(ARRAY[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20]) AS t(g);
ALTER TABLE actb ADD COLUMN clk TIMESTAMPTZ DEFAULT clock_timestamp();
ALTER TABLE actb ADD COLUMN nw TIMESTAMPTZ DEFAULT now();
ALTER TABLE actb ADD COLUMN tx TIMESTAMPTZ DEFAULT transaction_timestamp();
ALTER TABLE actb ADD COLUMN st TIMESTAMPTZ DEFAULT statement_timestamp();
ALTER TABLE actb ADD COLUMN cts TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP;
-- @test
SELECT count(DISTINCT clk) > 1 AS clock_is_per_row,
       count(DISTINCT nw) AS now_values,
       count(DISTINCT tx) AS transaction_values,
       count(DISTINCT st) AS statement_values,
       count(DISTINCT cts) AS current_timestamp_values
FROM actb;
