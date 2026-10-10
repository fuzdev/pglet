-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The keyless form of `rollback_of_a_drop_table_puts_back_the_serial_cell`: with no key
-- to refuse a repeat, a counter restored to the re-created cell's value re-issues ids
-- silently. 18.6 answers five rows with five distinct ids.
-- @setup
DROP TABLE IF EXISTS rdt_keyless;
CREATE TABLE rdt_keyless (id SERIAL, v TEXT);
INSERT INTO rdt_keyless (v) VALUES ('1'), ('2'), ('3');
BEGIN;
DROP TABLE rdt_keyless;
CREATE TABLE rdt_keyless (id SERIAL, v TEXT);
INSERT INTO rdt_keyless (v) VALUES ('x');
ROLLBACK;
INSERT INTO rdt_keyless (v) VALUES ('n1'), ('n2');
-- @test
SELECT count(*) AS n, count(DISTINCT id) AS distinct_ids, max(id) AS max_id FROM rdt_keyless;
