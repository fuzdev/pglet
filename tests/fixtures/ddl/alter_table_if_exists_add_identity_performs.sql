-- `ADD GENERATED … AS IDENTITY` under `IF EXISTS` makes the column draw its sequence.
-- @setup
CREATE TABLE atie8 (a INT NOT NULL, b TEXT);
ALTER TABLE IF EXISTS atie8 ALTER COLUMN a ADD GENERATED ALWAYS AS IDENTITY;
INSERT INTO atie8 (b) VALUES ('x');
-- @test
SELECT * FROM atie8;
