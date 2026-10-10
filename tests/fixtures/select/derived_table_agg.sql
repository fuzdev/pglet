-- @setup
CREATE TABLE sales (product TEXT, amount INT);
INSERT INTO sales VALUES ('A', 100), ('A', 150), ('B', 200), ('B', 50), ('C', 300);

-- @test
SELECT product, total FROM (SELECT product, SUM(amount) AS total FROM sales GROUP BY product) AS totals WHERE total > 100 ORDER BY product;
