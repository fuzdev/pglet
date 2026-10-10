-- A typed constant in the three non-`SELECT` positions a migration puts one in: a
-- column `DEFAULT`, a table `CHECK`, and an `INSERT … VALUES` entry. The `DEFAULT`
-- and the `CHECK` are **persisted** expressions, so this also pins that the node
-- deparses — an accepted expression that cannot be rendered makes the whole database
-- undumpable.
-- @setup
CREATE TABLE td (
  id INT,
  d DATE DEFAULT DATE '2024-01-01',
  n NUMERIC(5,2) DEFAULT NUMERIC '1.5',
  CHECK (d >= DATE '2020-01-01')
);
INSERT INTO td (id) VALUES (1);
INSERT INTO td (id, d) VALUES (2, DATE '2025-03-04');
-- @test
SELECT id, d, n FROM td ORDER BY id;
