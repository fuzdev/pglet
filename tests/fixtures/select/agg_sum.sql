-- @setup
CREATE TABLE sales (id INT, amount INT);
INSERT INTO sales VALUES (1, 100), (2, 200), (3, 150);

-- @test
SELECT SUM(amount) FROM sales;
