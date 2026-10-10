-- A sequence and an enum may share a name — the sequence first, since a sequence has no
-- row type — and `nextval('tnt40')` then names the sequence, at every default a table
-- cooks: `CREATE TABLE`, `ADD COLUMN` and `SET DEFAULT` are accepted on both engines.
-- The gate that refuses a `nextval` naming an enum asks the relation namespace first.
-- @setup
CREATE SEQUENCE tnt40;
CREATE TYPE tnt40 AS ENUM ('a');
CREATE TABLE tnt40t (id INT DEFAULT nextval('tnt40'), m tnt40);
ALTER TABLE tnt40t ADD COLUMN k INT DEFAULT nextval('tnt40');
ALTER TABLE tnt40t ADD COLUMN j INT;
ALTER TABLE tnt40t ALTER COLUMN j SET DEFAULT nextval('tnt40');
INSERT INTO tnt40t (m) VALUES ('a'), ('a');
-- @test
SELECT id, k, j, m::text AS m FROM tnt40t ORDER BY id;
