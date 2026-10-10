-- @setup
CREATE TABLE users (id INT, name TEXT);
CREATE TABLE orders (oid INT, uid INT, product TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob');
INSERT INTO orders VALUES (101, 1, 'Widget'), (102, 2, 'Gadget');

-- @test
SELECT * FROM users JOIN orders ON users.id = orders.uid ORDER BY users.id;
