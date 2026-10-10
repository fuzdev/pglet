-- @setup
CREATE TABLE trunc_t (id INT, name TEXT);
INSERT INTO trunc_t VALUES (1, 'a'), (2, 'b'), (3, 'c');
TRUNCATE trunc_t;

-- @test
SELECT * FROM trunc_t;
