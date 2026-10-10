-- @setup
CREATE TABLE users (id INT, name TEXT);
CREATE TABLE orders (id INT, user_id INT, amount INT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol');
INSERT INTO orders VALUES (101, 1, 100), (102, 1, 200), (103, 2, 150);

-- @test
SELECT name FROM users WHERE id IN (SELECT user_id FROM orders) ORDER BY name;
