-- @no-txn
-- Drives its own statements outside the harness's block, so each is its own transaction
-- (and it keeps its own DROP-first cleanup).
--
-- A `DO` is one statement: the statements of its body read one statement start, and —
-- run outside a block — one transaction start, which is the same instant. The wall
-- clock is read per call and is no earlier.
-- @setup
DROP TABLE IF EXISTS cdb;
CREATE TABLE cdb (id INT, tx TIMESTAMPTZ, stmt TIMESTAMPTZ, clk TIMESTAMPTZ, dflt TIMESTAMPTZ DEFAULT now());
DO $$ BEGIN INSERT INTO cdb (id, tx, stmt, clk) VALUES (1, now(), statement_timestamp(), clock_timestamp()); INSERT INTO cdb (id, tx, stmt, clk) VALUES (2, CURRENT_TIMESTAMP, statement_timestamp(), clock_timestamp()); UPDATE cdb SET tx = now() WHERE id = 1; END $$;
-- @test
SELECT count(*) AS body_rows,
       count(DISTINCT tx) AS transaction_values,
       count(DISTINCT stmt) AS statement_values,
       sum(CASE WHEN tx = stmt AND tx = dflt THEN 0 ELSE 1 END) AS columns_differing,
       sum(CASE WHEN stmt <= clk THEN 0 ELSE 1 END) AS clock_before_statement,
       max(tx) < now() AS a_later_statement_is_later
FROM cdb;
