-- `DEFAULT NULL` is no default: the identity clause is accepted over it.
-- @setup
CREATE TABLE aid (c INT NOT NULL DEFAULT NULL, v TEXT);
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
INSERT INTO aid (v) VALUES ('minted');
-- @test
SELECT c, v FROM aid;
