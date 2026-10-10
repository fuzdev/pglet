-- A bare parameter as an `OR` operand is `boolean` as soon as it is analysed — ahead
-- of the null test in the operand after it.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 bool NULL
-- @test
SELECT count(*) AS c FROM up_t WHERE $1 OR $1 IS NULL;
