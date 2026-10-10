-- The optional-filter idiom with the comparison written first: it types `$1` as `text`
-- before the null test is reached, so 18.6 answers. Bound NULL, every row passes.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text NULL
-- @test
SELECT id FROM up_t WHERE (name = $1 OR $1 IS NULL) ORDER BY id;
