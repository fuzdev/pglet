-- @setup
CREATE TABLE users (id INT, age INT);
INSERT INTO users VALUES (1, 25), (2, 35), (3, 45);

-- @test
SELECT * FROM users WHERE id > 1 AND age < 40 ORDER BY id;
