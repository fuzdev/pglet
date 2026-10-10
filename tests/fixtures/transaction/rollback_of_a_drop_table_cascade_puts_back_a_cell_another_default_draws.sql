-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `DROP TABLE … CASCADE` over a table whose owned sequence another table's `DEFAULT`
-- draws from: the drop takes the sequence and sheds that default. The rollback puts
-- both back, the cell at the value the draws left it, so the other table's next
-- default is `4` and the owner's next id `5` on 18.6.
-- @setup
DROP TABLE IF EXISTS rdt_dflt_q, rdt_dflt_p;
CREATE TABLE rdt_dflt_p (id SERIAL PRIMARY KEY);
CREATE TABLE rdt_dflt_q (x INT DEFAULT nextval('rdt_dflt_p_id_seq'), y INT);
INSERT INTO rdt_dflt_p DEFAULT VALUES;
INSERT INTO rdt_dflt_p DEFAULT VALUES;
INSERT INTO rdt_dflt_q (y) VALUES (1);
BEGIN;
DROP TABLE rdt_dflt_p CASCADE;
CREATE TABLE rdt_dflt_p (id SERIAL PRIMARY KEY);
INSERT INTO rdt_dflt_p DEFAULT VALUES;
ROLLBACK;
INSERT INTO rdt_dflt_q (y) VALUES (2);
-- @test
SELECT (SELECT max(x) FROM rdt_dflt_q) AS q_x, nextval('rdt_dflt_p_id_seq') AS p_next;
