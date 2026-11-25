-- `UPDATE ONLY (t)` and `DELETE FROM ONLY (t)` are `ONLY t`.
-- @setup
CREATE TABLE uop (id INT, v TEXT);
INSERT INTO uop VALUES (1, 'a'), (2, 'b');
DELETE FROM ONLY (uop) WHERE id = 2;
-- @test
UPDATE ONLY (uop) SET v = 'z' RETURNING id, v;
