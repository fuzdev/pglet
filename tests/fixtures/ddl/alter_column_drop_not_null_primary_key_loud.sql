-- A column of the primary key stays `NOT NULL` while the key stands: `42P16`, for a
-- composite key's second column too.
-- @expect-error: column is in a primary key
-- @expect-message
-- @no-profile-lock: the refusal reads the primary key from the catalog and selects no index
-- @setup
CREATE TABLE dnn (id int, k int, PRIMARY KEY (id, k));
-- @test
ALTER TABLE dnn ALTER COLUMN k DROP NOT NULL;
