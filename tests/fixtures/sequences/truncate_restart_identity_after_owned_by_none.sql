-- `TRUNCATE … RESTART IDENTITY` restarts only the truncated tables' **owned**
-- sequences, so a detached one keeps counting. This is the plain-SQL observable
-- `OWNED BY NONE` used to lack: with the restart set built from the columns' `NextVal`
-- defaults instead, the detach changed nothing and the next value was 1.
-- @setup
CREATE TABLE trin_t (a INT);
CREATE SEQUENCE trin_seq START WITH 1;
ALTER SEQUENCE trin_seq OWNED BY trin_t.a;
ALTER TABLE trin_t ALTER COLUMN a SET DEFAULT nextval('trin_seq');
INSERT INTO trin_t (a) VALUES (DEFAULT), (DEFAULT);
ALTER SEQUENCE trin_seq OWNED BY NONE;
TRUNCATE trin_t RESTART IDENTITY;
INSERT INTO trin_t (a) VALUES (DEFAULT);
-- @test
SELECT a FROM trin_t;
