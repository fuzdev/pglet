-- The candidate the key names fails the residual: no row is written.
-- @profile-lock
-- @param $1 int4 2
-- @param $2 text zz
-- @setup
CREATE TABLE pq (id INT PRIMARY KEY, v TEXT);
INSERT INTO pq VALUES (1, 'a'), (2, 'b');
-- @test
UPDATE pq SET v = 'x' WHERE id = $1 AND v = $2 RETURNING id, v;
