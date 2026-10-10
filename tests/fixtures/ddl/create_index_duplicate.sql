-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE users (id INT, name TEXT, email TEXT);
CREATE INDEX idx_users_email ON users(email);

-- @test
-- @expect-error: relation "idx_users_email" already exists
CREATE INDEX idx_users_email ON users(email);
