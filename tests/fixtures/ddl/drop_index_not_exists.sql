-- @setup
CREATE TABLE users (id INT, name TEXT, email TEXT);

-- @test
-- @expect-error: index "idx_nonexistent" does not exist
DROP INDEX idx_nonexistent;
