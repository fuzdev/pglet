-- Every column of the dropped table is inside the deletion set, so a table whose own
-- columns draw from its own sequence drops cleanly — including a *sibling* column
-- naming the SERIAL's sequence, which blocks a column drop and not a table drop. The
-- accept side an over-correction reddens.
-- @setup
CREATE TABLE dtod_t (a SERIAL, b INT DEFAULT nextval('dtod_t_a_seq'));
INSERT INTO dtod_t (b) VALUES (7);
DROP TABLE dtod_t;
CREATE TABLE dtod_t (a SERIAL, b INT);
INSERT INTO dtod_t (b) VALUES (7);
-- @test
SELECT a, b FROM dtod_t;
