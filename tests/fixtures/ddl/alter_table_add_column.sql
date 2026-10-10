-- @setup
CREATE TABLE users (id INT, name TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob');
ALTER TABLE users ADD COLUMN email TEXT;
INSERT INTO users VALUES (3, 'Carol', 'carol@test.com');

-- @test
SELECT * FROM users ORDER BY id;
