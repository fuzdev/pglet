-- @setup
CREATE TABLE users (id INT, name TEXT);
CREATE TABLE orders (id INT, user_id INT, product TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol');
INSERT INTO orders VALUES (101, 1, 'Widget'), (102, 1, 'Gadget'), (103, 2, 'Gizmo');

-- @test
SELECT users.name, orders.product FROM users LEFT JOIN orders ON users.id = orders.user_id ORDER BY users.name, orders.product;
