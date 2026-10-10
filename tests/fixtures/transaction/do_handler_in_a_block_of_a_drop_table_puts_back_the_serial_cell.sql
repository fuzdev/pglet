-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The caught `DO` of `do_handler_of_a_drop_table_puts_back_the_serial_cell` inside a
-- block: the caught body's rollback puts the old cell back at `3`, the block draws `4`, and
-- the block's rollback leaves that draw standing, so 18.6 answers `5`.
-- @setup
DROP TABLE IF EXISTS rdo_block;
CREATE TABLE rdo_block (id SERIAL PRIMARY KEY, v TEXT);
INSERT INTO rdo_block (v) VALUES ('1'), ('2'), ('3');
BEGIN;
DO $$ BEGIN DROP TABLE rdo_block; CREATE TABLE rdo_block (id SERIAL PRIMARY KEY, v TEXT); INSERT INTO rdo_block (v) VALUES ('x'); INSERT INTO rdo_block (id, v) VALUES (1, 'dup'); EXCEPTION WHEN OTHERS THEN NULL; END $$;
INSERT INTO rdo_block (v) VALUES ('in-block') RETURNING id;
ROLLBACK;
-- @test
INSERT INTO rdo_block (v) VALUES ('n1') RETURNING id;
