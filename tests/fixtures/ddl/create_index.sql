-- @setup
CREATE TABLE users (id INT, name TEXT, email TEXT);

-- @test
CREATE INDEX idx_users_email ON users(email);
