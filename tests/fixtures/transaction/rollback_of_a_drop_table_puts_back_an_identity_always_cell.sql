-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The identity form of `rollback_of_a_drop_table_puts_back_the_serial_cell`: an identity
-- column's sequence is owned by it and dropped with the table, and 18.6 answers `4`
-- and `5` after the rollback.
-- @setup
DROP TABLE IF EXISTS rdt_ident_a;
CREATE TABLE rdt_ident_a (id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, v TEXT);
INSERT INTO rdt_ident_a (v) VALUES ('1'), ('2'), ('3');
BEGIN;
DROP TABLE rdt_ident_a;
CREATE TABLE rdt_ident_a (id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, v TEXT);
INSERT INTO rdt_ident_a (v) VALUES ('x');
ROLLBACK;
-- @test
INSERT INTO rdt_ident_a (v) VALUES ('n1'), ('n2') RETURNING id;
