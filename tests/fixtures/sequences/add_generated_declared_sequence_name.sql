-- The `ALTER … ADD GENERATED` route — the form pg_dump emits, and the one where
-- discarding the name silently restarts a restored identity counter at 1 on top of
-- the restored rows. (The column is declared NOT NULL first, as both engines require.)
-- @setup
CREATE TABLE agdsn (id INT NOT NULL);
ALTER TABLE agdsn ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME agdsn_custom);
SELECT setval('agdsn_custom', 7, true);
INSERT INTO agdsn (id) VALUES (DEFAULT);
-- @test
SELECT id FROM agdsn;
