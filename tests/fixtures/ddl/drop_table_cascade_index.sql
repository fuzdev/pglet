-- @setup
CREATE TABLE users (id INT, name TEXT, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_name ON users(name);

-- @test
-- Verify DROP TABLE removes associated indexes
-- First drop the table
DROP TABLE users;
-- Then try to drop the index - should fail because table (and its indexes) are gone
DROP INDEX idx_users_email;
