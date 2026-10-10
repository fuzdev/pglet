-- @setup
CREATE TABLE dq (id INT, x INT);
INSERT INTO dq VALUES (1, 30), (2, 10), (3, 20), (4, 10);

-- A qualified key spelled against the FROM alias denotes the projected column.
-- @test
SELECT DISTINCT x FROM dq AS c ORDER BY c.x;
