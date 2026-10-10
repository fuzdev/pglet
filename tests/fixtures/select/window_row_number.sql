-- @setup
CREATE TABLE employees (id INT, name TEXT, department TEXT, salary INT);
INSERT INTO employees VALUES
  (1, 'Alice', 'Engineering', 80000),
  (2, 'Bob', 'Engineering', 75000),
  (3, 'Carol', 'Sales', 60000),
  (4, 'Dave', 'Sales', 65000),
  (5, 'Eve', 'Engineering', 90000);

-- @test
SELECT name, department, salary, ROW_NUMBER() OVER (ORDER BY salary DESC) AS rank
FROM employees
ORDER BY rank;
