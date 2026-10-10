-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating wrapper.
--
-- The under-claim polarity of the fixture beside it, and the reason
-- `Plan::swapped_sequences` reads the truncation **closure** rather than the named
-- tables. `rricc_p` owns no sequence; CASCADE pulls in `rricc_c`, whose sequence
-- `RESTART IDENTITY` does restart (`ddl/truncate_restart_identity_cascade_child.sql`
-- pins that outside a transaction). Reading only the named tables would record no
-- replaced cell here and let the restart outlive its own ROLLBACK — the next id would
-- be 7 rather than 9.
--
-- `START WITH 7` is load-bearing for the same reason as in
-- `rollback_truncate_restart_identity.sql`: with the default start, a restarted counter
-- and a rolled-back one both hand out small numbers and the read cannot tell them apart.
-- @setup
CREATE TABLE rricc_p (id INT PRIMARY KEY);
CREATE TABLE rricc_c (id INT GENERATED ALWAYS AS IDENTITY (START WITH 7), pid INT REFERENCES rricc_p(id));
INSERT INTO rricc_p (id) VALUES (1);
INSERT INTO rricc_c (pid) VALUES (1), (1);
BEGIN;
TRUNCATE rricc_p RESTART IDENTITY CASCADE;
ROLLBACK;
INSERT INTO rricc_c (pid) VALUES (1);
-- @test
SELECT id, pid FROM rricc_c ORDER BY id;
