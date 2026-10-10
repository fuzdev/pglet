-- `SET DEFAULT NULL` removes a default, and the identity clause is accepted after it.
-- @setup
CREATE TABLE aid (c INT NOT NULL DEFAULT 7, v TEXT);
ALTER TABLE aid ALTER COLUMN c SET DEFAULT NULL;
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
INSERT INTO aid (v) VALUES ('minted');
-- @test
SELECT c, v FROM aid;
