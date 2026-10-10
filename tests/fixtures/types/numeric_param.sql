-- A bound NUMERIC parameter over the extended protocol: $1's type is inferred
-- from the `amount` column, decoded from the wire numeric, and the row matches.
-- @setup
CREATE TABLE prices (id INT, amount NUMERIC(10, 2));
INSERT INTO prices VALUES (1, 19.99), (2, 9.50), (3, 100.00);

-- @param $1 numeric 9.50
-- @test
SELECT id FROM prices WHERE amount = $1;
