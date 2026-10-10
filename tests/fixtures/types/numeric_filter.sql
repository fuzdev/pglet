-- A comparison literal coerces to NUMERIC so the filter matches by value
-- (1.50 == 1.5), and a decimal literal comparison works too.
-- @setup
CREATE TABLE prices (id INT, amount NUMERIC(10, 2));
INSERT INTO prices VALUES (1, 19.99), (2, 9.50), (3, 100.00), (4, 9.5);

-- @test
SELECT id FROM prices WHERE amount = 9.5 OR amount > 50 ORDER BY id;
