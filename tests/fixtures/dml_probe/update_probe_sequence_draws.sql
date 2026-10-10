-- An equality-probed UPDATE draws its sequence values in row order: `SET n = DEFAULT` over
-- a serial column draws 4, 5, 6 for the three k = 1 rows in the order they were inserted.
-- @profile-lock
-- @setup
CREATE TABLE psd (k INT, tag TEXT, n SERIAL);
CREATE INDEX psd_k ON psd (k);
INSERT INTO psd (k, tag) VALUES (1, 'c'), (1, 'a'), (1, 'b');
-- @test
UPDATE psd SET n = DEFAULT WHERE k = 1 RETURNING tag, n;
