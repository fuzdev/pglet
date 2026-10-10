-- A relation the catalog lacks is `42P01` whatever the statement's parameters: 18.6
-- resolves the range table before it looks at one. The null test is ahead of the
-- comparison that types `$1`, which over a relation that exists is `42P08`.
-- @setup
CREATE TABLE up_t (id INT, name TEXT, n BIGINT);
INSERT INTO up_t VALUES (1, 'x', 10), (2, 'y', 20), (3, NULL, 30);
-- @param $1 text x
-- @test
-- @expect-error: the missing relation is reported ahead of the parameter: 42P01
INSERT INTO up_nope (id) VALUES (1) RETURNING $1 IS NULL AND $1 = 'x'::text;
