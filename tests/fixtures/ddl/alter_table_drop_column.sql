-- @setup
CREATE TABLE users (id INT, name TEXT, email TEXT);
INSERT INTO users VALUES (1, 'Alice', 'alice@test.com'), (2, 'Bob', 'bob@test.com');
ALTER TABLE users DROP COLUMN email;

-- @test
SELECT * FROM users ORDER BY id;
