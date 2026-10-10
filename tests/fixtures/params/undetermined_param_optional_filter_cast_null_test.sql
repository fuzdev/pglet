-- The null test written first over a cast parameter: the cast types `$1`, so the order
-- of the two operands stops mattering.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT id FROM up_t WHERE ($1::text IS NULL OR name = $1) ORDER BY id;
