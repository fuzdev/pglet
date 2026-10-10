-- An accepted cross-type key keeps its actions: a `bigint` column referencing an `int` key
-- `ON UPDATE CASCADE` follows the key.
-- @setup
CREATE TABLE ftp (k INT PRIMARY KEY);
CREATE TABLE ftc (b BIGINT REFERENCES ftp ON UPDATE CASCADE);
INSERT INTO ftp VALUES (1), (2);
INSERT INTO ftc VALUES (1), (2);

-- @test
UPDATE ftp SET k = k + 10;
SELECT b FROM ftc ORDER BY b;
