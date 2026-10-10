-- @setup
CREATE TABLE items (id INT, data JSONB);
INSERT INTO items VALUES (1, '{"name": "Alice", "age": 30}');

-- @test
SELECT data->>'name' AS name FROM items;
