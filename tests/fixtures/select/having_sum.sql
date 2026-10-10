-- @setup
CREATE TABLE sales (id INT, region TEXT, amount FLOAT);
INSERT INTO sales VALUES
  (1, 'north', 100.0),
  (2, 'north', 200.0),
  (3, 'south', 50.0),
  (4, 'east', 300.0),
  (5, 'east', 400.0),
  (6, 'west', 25.0);

-- @test
SELECT region, SUM(amount) FROM sales GROUP BY region HAVING SUM(amount) > 100 ORDER BY region;
