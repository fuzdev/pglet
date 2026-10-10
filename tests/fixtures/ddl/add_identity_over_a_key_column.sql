-- A primary key's column is `NOT NULL` by the key, and takes the identity clause.
-- @profile-lock
-- @setup
CREATE TABLE ai (c INT PRIMARY KEY, v TEXT);
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
INSERT INTO ai (v) VALUES ('first'), ('second');
-- @test
SELECT c, v FROM ai ORDER BY c;
