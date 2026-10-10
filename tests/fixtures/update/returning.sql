-- @setup
CREATE TABLE users (id INT, name TEXT, active BOOL);
INSERT INTO users VALUES (1, 'Alice', false), (2, 'Bob', false), (3, 'Carol', true);

-- @test
UPDATE users SET active = true WHERE id = 1 RETURNING id, name, active;
