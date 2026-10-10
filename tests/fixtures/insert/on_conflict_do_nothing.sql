-- @setup
CREATE TABLE users (id INT UNIQUE, name TEXT);
INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob');
INSERT INTO users VALUES (1, 'NewAlice'), (3, 'Carol')
ON CONFLICT (id) DO NOTHING;

-- @test
SELECT * FROM users ORDER BY id;
