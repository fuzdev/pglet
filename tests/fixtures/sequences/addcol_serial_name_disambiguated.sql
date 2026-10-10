-- `ALTER TABLE ... ADD COLUMN <c> SERIAL` mints through the same namer, so a taken
-- name disambiguates there too.
-- @setup
CREATE TABLE addser (x INT);
INSERT INTO addser VALUES (1);
CREATE SEQUENCE addser_y_seq START WITH 500;
ALTER TABLE addser ADD COLUMN y SERIAL;
-- @test
SELECT x, y FROM addser;
