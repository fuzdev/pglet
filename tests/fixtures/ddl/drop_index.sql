-- @setup
CREATE TABLE users (id INT, name TEXT, email TEXT);
CREATE INDEX idx_users_email ON users(email);

-- @test
DROP INDEX idx_users_email;
