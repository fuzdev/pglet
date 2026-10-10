-- @setup
CREATE TABLE items (id INT, name TEXT);
INSERT INTO items VALUES (1, 'Apple'), (2, 'Banana'), (3, 'Cherry');

-- @test
SELECT COUNT(*) FROM items;
