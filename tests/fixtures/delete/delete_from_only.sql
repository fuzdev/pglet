-- `DELETE FROM ONLY t` deletes from `t`: no table inherits from it.
-- @setup
CREATE TABLE dfo (id INT);
INSERT INTO dfo VALUES (1), (2);
-- @test
DELETE FROM ONLY dfo WHERE id = 2 RETURNING id;
