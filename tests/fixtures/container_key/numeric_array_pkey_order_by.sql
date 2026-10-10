-- A `numeric[]` primary key orders by its elements' values: `NaN` above every number,
-- a negative below zero, scale not part of the order.
-- @profile-lock
-- @setup
CREATE TABLE cko_n (id INT, a NUMERIC[] PRIMARY KEY);
INSERT INTO cko_n (a, id) VALUES ('{1.50}', 1), ('{NaN}', 2), ('{0.00}', 3), ('{-2}', 4), ('{1.5,0}', 5), ('{10}', 6), ('{1.25}', 7), ('{}', 8), ('{-2.5}', 9);
-- @test
SELECT id, a FROM cko_n ORDER BY a;
