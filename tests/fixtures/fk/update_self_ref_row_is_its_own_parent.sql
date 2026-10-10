-- A row that references its own key `ON UPDATE CASCADE` follows its own move.
-- @setup
CREATE TABLE ss (a int PRIMARY KEY, b int UNIQUE REFERENCES ss (a) ON UPDATE CASCADE);
INSERT INTO ss VALUES (1, NULL);
UPDATE ss SET b = 1;
-- @test
-- @profile-lock
UPDATE ss SET a = 9;
SELECT a, b FROM ss;
