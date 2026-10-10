-- A SERIAL (nextval) default backfills sequential values across existing rows; the
-- set {1,2,3} is deterministic even though per-row assignment order may vary.
-- @setup
CREATE TABLE alter_addcol_ser (id INT);
INSERT INTO alter_addcol_ser VALUES (1), (2), (3);
ALTER TABLE alter_addcol_ser ADD COLUMN s SERIAL;

-- @test
SELECT count(DISTINCT s), min(s), max(s) FROM alter_addcol_ser;
