-- @setup
CREATE TABLE users (id INT, active BOOL);
INSERT INTO users VALUES (1, false), (2, false);
UPDATE users SET active = true;

-- @test
SELECT * FROM users ORDER BY id;
