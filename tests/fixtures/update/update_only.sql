-- `UPDATE ONLY t` updates `t`: no table inherits from it.
-- @setup
CREATE TABLE uo (id INT, v TEXT);
INSERT INTO uo VALUES (1, 'a'), (2, 'b');
-- @test
UPDATE ONLY uo SET v = 'z' WHERE id = 1 RETURNING id, v;
