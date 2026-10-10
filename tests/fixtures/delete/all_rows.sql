-- @setup
CREATE TABLE users (id INT);
INSERT INTO users VALUES (1), (2), (3);
DELETE FROM users;

-- @test
SELECT * FROM users;
