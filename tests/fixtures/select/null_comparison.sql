-- @setup
CREATE TABLE items (id INT, name TEXT);
INSERT INTO items VALUES (1, 'Apple'), (2, NULL), (3, 'Cherry');

-- @test
SELECT * FROM items WHERE name = 'Apple' ORDER BY id;
