-- A plain column `DEFAULT nextval(...)` — what a `SERIAL` carries — is the NORMAL
-- `pg_attrdef` edge: refused without CASCADE.
-- @setup
CREATE TABLE dscd_t (id SERIAL, b INT);
-- @test
DROP SEQUENCE dscd_t_id_seq;
-- @expect-error: a column default depends on the sequence (2BP01)
