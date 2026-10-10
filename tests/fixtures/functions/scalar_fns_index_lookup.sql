-- A lookup over an expression index on `left` and one on `md5` answers the rows a scan
-- does.
-- @setup
CREATE TABLE il (id INT, t TEXT);
INSERT INTO il VALUES (1, 'apple'), (2, 'apricot'), (3, 'banana'), (4, NULL);
CREATE INDEX il_l ON il (left(t, 2));
CREATE INDEX il_m ON il (md5(t));
-- @test
SELECT id FROM il WHERE left(t, 2) = 'ap' OR md5(t) = md5('banana') ORDER BY id;
