-- @setup
CREATE TABLE users (id INT, age INT);
INSERT INTO users VALUES (1, 25), (2, 35), (3, 45);

-- @test
SELECT * FROM users WHERE age < 30 OR age > 40 ORDER BY id;
